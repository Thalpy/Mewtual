//! The **product model**; a UI-facing facade over the Mewtual stack.
//!
//! Everything below `catcoms-sync` speaks in protocol terms (MLS groups, sealed ops,
//! automerge docs, transport peers). A UI does not want any of that; it wants
//! *servers*, *channels*, *messages*, and *members*. This crate is that translation
//! layer: a thin, typed boundary the desktop/mobile UI (and the Tauri command bridge)
//! is built against, so the GUI never touches MLS or automerge directly.
//!
//! It owns the **canonical channel-message schema** (a channel document is a list of
//! `{author, text}` maps); previously poked inline in `catcomsctl`, now defined once
//! here so the protocol and the UI agree on what a chat message is.
//!
//! Scope (slice 8a): a single [`Server`] (one [`ChannelSync`]), driven explicitly via
//! [`Server::sync_once`]. The background run-loop + live event stream and multi-server
//! management land with the Tauri bridge (8b), where the real async runtime lives.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use automerge::transaction::Transactable;
use automerge::{
    AutoCommit, AutomergeError, ChangeHash, ObjId, ObjType, ReadDoc, ScalarValue, Value, ROOT,
};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
// Re-export the device-identity type: it is part of the app's public surface (the
// roster, the grant ceremony) and the bridge names it to drive pairing.
pub use catcoms_crypto::DeviceId;
// The companion-device statements the `Devices` registry stores + verifies (multi-device M3).
use catcoms_crypto::verify_with_public_bytes;
pub use catcoms_crypto::{DeviceCertificate, DeviceRevocation};
use catcoms_mls::{InviteToken, MlsDevice, MlsError, ServerGroup};
use catcoms_rt::{
    Clock, CryptoRngCore, DiscoveredPeer, MeshTransport, PeerId, RequestCancellation,
};
use catcoms_storage::{BlobStore, FileManifest};
// Re-export the content-address type: it's part of the app's public file surface, and the bridge
// verifies a reassembled download against it.
pub use catcoms_storage::{Cid, CidHasher, FileRef, SealedBlob};
use catcoms_sync::{
    fingerprint, read_published_roster, request_device_join_tracking,
    request_join_from_reply_tracking, request_join_from_switchboards_tracking,
    request_join_tracking, request_join_via_helper_tracking, ChannelSync,
    PreOwnerConnectionHandoff, SyncError, ROLES_DOC,
};
pub use catcoms_sync::{
    peer_addrs_from_snapshot, InviteJoinPlan, JoinAttempt, JoinOutcome, MemberRecoveryApplied,
    MemberRecoveryCode, MemberRecoveryVerified, SwitchboardOffer, SwitchboardRoute,
    MEMBER_RECOVERY_LIFETIME_MS, SWITCHBOARD_OFFER_LIFETIME_MS, SWITCHBOARD_OFFER_MAX_FUTURE_MS,
};
use catcoms_wire::DocType;
use thiserror::Error;

mod actor;
mod moderation;
pub mod pairing;
pub mod store;
pub use actor::{spawn, AppCommand, AppEvent, Envelope, ServerActor, Trace, TracedEvent};
pub use moderation::{
    ModerationEvent, ModerationState, ModerationVote, MAX_MOD_EVIDENCE_BYTES, MAX_MOD_EVIDENCE_IDS,
    MAX_MOD_REASON_BYTES,
};
pub use pairing::{
    begin_pairing, decode_pairing_blob, mint_grant_bundle, open_grant_bundle, read_pairing_blob,
    OpenedGrantBundle, PairingLedger, PairingRequestView, PairingSecrets, PerServerGrant,
    GRANT_BLOB_PREFIX, PAIRING_BLOB_PREFIX,
};
// The companion device identity a grant is redeemed with (multi-device M3): the bridge holds one
// `PairingSecrets` and duplicates its device per granted server.
pub use catcoms_mls::MlsDevice as PairedDevice;
pub use store::{
    ReconnectPolicy, ReconnectRoute, ServerNet, ServerRecord, ServerStore, MAX_RECONNECT_ROUTES,
    MAX_RECONNECT_ROUTE_BYTES,
};

/// Errors surfaced to the UI/product layer.
#[derive(Debug, Error)]
pub enum AppError {
    /// A channel-sync / transport error.
    #[error(transparent)]
    Sync(#[from] SyncError),
    /// An MLS-layer error (e.g. founding a group).
    #[error(transparent)]
    Mls(#[from] MlsError),
    /// A keystore (seal/unseal) error at rest.
    #[error(transparent)]
    Keystore(#[from] catcoms_crypto::KeystoreError),
    /// A storage-layer error (e.g. opening the on-disk vault).
    #[error(transparent)]
    Storage(#[from] catcoms_storage::StorageError),
    /// A persistence I/O error (reading/writing the on-disk store).
    #[error("persistence i/o: {0}")]
    Io(String),
    /// The destination rename committed, but flushing its directory failed. The caller must not
    /// report a clean rollback: the new complete record is visible, although it may not survive
    /// sudden power loss. Retrying the same save is safe and reconciles the durability state.
    #[error("persistence committed but could not be made durable: {0}")]
    CommittedButNotDurable(String),
    /// A product-layer validation error (e.g. an over-large avatar).
    #[error("{0}")]
    Invalid(String),
    /// The join did not finalize before the wall-clock deadline; e.g. an Option-C admin invite
    /// whose owner never came online to serialize the Add. The user can retry.
    #[error("the join timed out before it finalized; try again")]
    JoinTimeout,
}

/// One chat message as the UI sees it. The `author` is the sender's **device
/// fingerprint** (the key its [`Profile`] is stored under); the UI resolves it to a
/// display name + styling at render time, so a profile change updates all of that
/// member's messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// A stable per-message id (random hex), for addressing edits/deletes under concurrent merges.
    /// Empty for legacy messages sent before ids existed (those can't be edited/deleted).
    pub id: String,
    /// The author's device fingerprint (resolve to a name/style via [`Server::profiles`]).
    pub author: String,
    /// The message text.
    pub text: String,
    /// Send time in epoch-millis (the sender's injected clock; `0` if absent).
    pub ts: u64,
    /// Wall-clock of the last edit (epoch-millis), or `0` if never edited.
    pub edited: u64,
    /// Emoji reactions on this message (empty if none).
    pub reactions: Vec<Reaction>,
    /// The id of the message this one replies to, or empty if it isn't a reply.
    pub reply_to: String,
    /// Whether this message is pinned in its channel.
    pub pinned: bool,
}

/// One channel's message list materialized at one document version, plus the lookups every
/// paged read needs and that would otherwise be rebuilt per read: position by id (page anchors,
/// reply parents) and the number of replies each message has received. Built in one pass by
/// [`MaterializedChannel::build`]; see `Server::messages_cache`.
#[derive(Debug, Default)]
struct MaterializedChannel {
    /// The [`Server::doc_version`] these rows were read at; `u64::MAX` until the first read.
    version: u64,
    messages: Vec<ChatMessage>,
    /// Position of each message with a non-empty id. Legacy id-less rows are not addressable.
    by_id: HashMap<String, usize>,
    /// How many messages name each id as their `reply_to`.
    reply_counts: HashMap<String, u32>,
}

impl MaterializedChannel {
    fn build(version: u64, messages: Vec<ChatMessage>) -> Self {
        let mut by_id = HashMap::with_capacity(messages.len());
        let mut reply_counts: HashMap<String, u32> = HashMap::new();
        for (index, m) in messages.iter().enumerate() {
            if !m.id.is_empty() {
                // First occurrence wins: an id can only repeat through a malformed or replayed
                // write, and the earlier row is the one every other member also saw first.
                by_id.entry(m.id.clone()).or_insert(index);
            }
            if !m.reply_to.is_empty() {
                *reply_counts.entry(m.reply_to.clone()).or_default() += 1;
            }
        }
        Self {
            version,
            messages,
            by_id,
            reply_counts,
        }
    }
}

/// Where a paged read of a channel is centred. Ids are the durable anchor (they survive
/// concurrent inserts, edits and deletes around them); an index is the fallback for a row that
/// has no id or was deleted; the tail is where a fresh open starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageAnchor {
    /// The newest row.
    Tail,
    /// The row with this message id.
    Id(String),
    /// The row at this position in the current order (clamped to the last row).
    Index(u64),
    /// The first row that replies to this message id (the "N replies" jump).
    FirstReplyTo(String),
}

/// A bounded read: the anchor row, up to `before` rows above it and `after` rows below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagePageQuery {
    pub anchor: PageAnchor,
    pub before: usize,
    pub after: usize,
    /// When set, the page also carries an [`UnreadSummary`] measured against this boundary.
    pub unread: Option<UnreadProbe>,
}

/// The read boundary a client wants unread state measured against, plus its own clock so the
/// plausibility ceiling is computed the way the client's read marks are (see `readCeiling` in
/// the desktop's `unread.ts`, whose rule this mirrors exactly).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadProbe {
    /// Effective timestamp of the newest row the client had read when it opened the channel;
    /// `u64::MAX` for "everything is read".
    pub divider_ts: u64,
    /// The client's wall clock (epoch-millis).
    pub now_ms: u64,
}

/// How far ahead of the client's clock a sender's timestamp may be and still be believed;
/// the same five minutes as the desktop's `CLOCK_SKEW_GRACE_MS`.
pub const CLOCK_SKEW_GRACE_MS: u64 = 5 * 60_000;

/// Unread state for a whole channel, measured natively so a client holding one page of it can
/// still place the divider and count what sits past it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnreadSummary {
    /// The newest timestamp in the channel within the clock grace: every row's timestamp is
    /// pulled down to this before it is compared, so one broken or hostile clock cannot park the
    /// boundary in the future.
    pub ceiling_ts: u64,
    /// Position of the first row from somebody else past the boundary, if any.
    pub first_index: Option<u64>,
    /// Rows from somebody else past the boundary.
    pub count: u64,
}

/// One row of a paged read, with what the row needs from the rest of the channel resolved
/// natively so the webview never has to hold the history to render it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagedMessage {
    pub message: ChatMessage,
    /// An `@[my name]` mention or a reply to one of my messages (see [`Server::message_tail`]).
    pub targets_me: bool,
    /// How many messages reply to this one, across the whole channel.
    pub reply_count: u32,
    /// The parent this row replies to, when it can be found.
    pub reply_to_preview: Option<ReplyPreview>,
}

/// Enough of a reply's parent to render the quote line: who wrote it and the start of its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyPreview {
    pub id: String,
    pub author: String,
    /// The parent's text, cut to [`REPLY_PREVIEW_CHARS`] characters.
    pub text: String,
}

/// Characters of a reply parent's text carried in a [`ReplyPreview`].
pub const REPLY_PREVIEW_CHARS: usize = 200;

/// The result of a paged read: a contiguous slice of the channel in its current order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MessagePage {
    /// The document version the rows were read at ([`Server::doc_version`]). Two pages at the
    /// same version are slices of the same list and can be stitched by index.
    pub version: u64,
    /// Rows in the whole channel.
    pub total: u64,
    /// Position of `rows[0]` in the whole channel (meaningless when `rows` is empty).
    pub start: u64,
    /// Position the anchor resolved to; `None` when an id anchor names no current row, in which
    /// case `rows` is empty and the caller chooses another anchor.
    pub anchor_index: Option<u64>,
    pub rows: Vec<PagedMessage>,
    /// Present when the query carried an [`UnreadProbe`].
    pub unread: Option<UnreadSummary>,
}

/// One emoji reaction on a message: the emoji plus the fingerprints of the members who reacted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaction {
    /// The emoji (a short string; a unicode emoji, or `:name:` for a custom one).
    pub emoji: String,
    /// Fingerprints of the members who added this reaction (deduped; order not significant).
    pub by: Vec<String>,
}

/// The newest activity in one channel, without its text. Enough for a client to rebuild unread
/// state from its own durable read marks after an explicit lock, a restart or an offline
/// catch-up, none of which can be reconstructed from a live event stream that was not running.
/// See [`Server::channel_heads`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChannelHead {
    /// The channel this head describes.
    pub channel: u128,
    /// Total messages in the channel.
    pub count: u64,
    /// Newest message timestamp of any author (epoch-millis), or `0` if the channel is empty.
    pub latest_ts: u64,
    /// Newest timestamp among messages this device did NOT write, or `0` if there are none.
    /// Own messages never make a channel unread, so this is the value a read mark is measured
    /// against.
    pub latest_incoming_ts: u64,
    /// The id of the message `latest_incoming_ts` came from (empty when there is none, or when
    /// the message predates stable ids).
    pub latest_incoming_id: String,
}

/// Lightweight activity stats over a conversation (no message text), for the friends-list
/// sortings. See [`Server::message_stats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MessageStats {
    /// Total messages in the channel.
    pub count: u64,
    /// Earliest message timestamp (epoch-millis), or `0` if none.
    pub first_ts: u64,
    /// Latest message timestamp (epoch-millis), or `0` if none.
    pub last_ts: u64,
    /// Distinct UTC days on which a message was sent.
    pub active_days: u64,
}

/// Normalize a display name into the form the desktop UI carries inside an `@[Name]` mention marker
/// (mirrors `mentionName` there): swap the bracket/newline chars that would break the marker for a
/// space, collapse runs of whitespace, and bound the length to the marker's 40-char cap. Inbox
/// mention-detection and the composer's insertion must agree, or mentions are silently missed.
pub(crate) fn normalize_mention_name(name: &str) -> String {
    let swapped: String = name
        .chars()
        .map(|c| {
            if matches!(c, '[' | ']' | '\n') {
                ' '
            } else {
                c
            }
        })
        .collect();
    swapped
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(40)
        .collect()
}

/// One mention/reply addressed to the local member, materialized for the cross-server inbox. A
/// single message can be both a mention and a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxItem {
    /// The channel the message is in.
    pub channel: u128,
    /// The message's stable id (for jump-to).
    pub message_id: String,
    /// The author's fingerprint.
    pub author: String,
    /// The author's display name in this server (resolved here since names are per-server).
    pub author_name: String,
    /// The message text.
    pub text: String,
    /// Send time (epoch-millis).
    pub ts: u64,
    /// The message `@[my name]`-mentions me.
    pub mention: bool,
    /// The message replies to one of my messages.
    pub reply: bool,
}

/// Deterministically derive a channel's document id from its **name**, so any two
/// members who open the same channel name converge on the same channel; IRC-style name
/// addressing, with no shared channel registry. Names are normalized (trimmed +
/// lowercased), so "General" and " general " address the same channel. The id is scoped
/// to the channel name only; the *group* scoping happens at the topic layer (the topic
/// mixes in the group's `ns_secret_L` + `group_id`), so the same name in different
/// servers stays isolated.
pub fn channel_id(name: &str) -> u128 {
    let mut h = blake3::Hasher::new();
    h.update(b"catcoms/channel-name/v1");
    h.update(name.trim().to_lowercase().as_bytes());
    let bytes = h.finalize();
    u128::from_be_bytes(bytes.as_bytes()[..16].try_into().expect("16 bytes"))
}

/// One entry in the server-wide channel directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInfo {
    pub id: u128,
    pub name: String,
}

/// The channel directory is one small, shared CRDT document; message content remains split into
/// the per-channel documents addressed by [`channel_id`].
const CHANNEL_INDEX_DOC: u128 = 0;
pub const MAX_CHANNEL_NAME_CHARS: usize = 64;

fn validate_channel_name(name: &str) -> Result<String, AppError> {
    let display = name.trim();
    if display.is_empty() {
        return Err(AppError::Invalid("a channel needs a name".into()));
    }
    if display.chars().count() > MAX_CHANNEL_NAME_CHARS {
        return Err(AppError::Invalid(format!(
            "channel name too long (max {MAX_CHANNEL_NAME_CHARS} characters)"
        )));
    }
    if display.chars().any(char::is_control) {
        return Err(AppError::Invalid(
            "channel names cannot contain control characters".into(),
        ));
    }
    Ok(display.to_string())
}

fn read_channel_index(doc: &AutoCommit) -> Vec<ChannelInfo> {
    let mut out = Vec::new();
    for key in doc.keys(ROOT) {
        let Ok(id) = u128::from_str_radix(&key, 16) else {
            continue;
        };
        let name = str_field(doc, &ROOT, &key);
        // Do not let a malformed or hostile catalog entry redirect a human-readable name to a
        // different document. The id is entirely derived from that name.
        if !name.is_empty() && channel_id(&name) == id {
            out.push(ChannelInfo { id, name });
        }
    }
    let general = ChannelInfo {
        id: channel_id("general"),
        name: "general".into(),
    };
    if !out.iter().any(|c| c.id == general.id) {
        // Backwards compatibility for servers created before the shared directory existed.
        out.push(general);
    }
    out.sort_by(|a, b| {
        (a.id != channel_id("general"))
            .cmp(&(b.id != channel_id("general")))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out.dedup_by_key(|c| c.id);
    out
}

/// Wall-clock ceiling on a join handshake. Past this the joiner gives up (and the user can
/// retry); so an Option-C admin invite whose owner never comes online can't wedge it forever.
const JOIN_TIMEOUT_SECS: u64 = 120;

// --- the canonical channel-document schema ----------------------------------
// A channel doc is `{ topic: str, messages: [ { author: str, text: str } ] }`.

const MESSAGES: &str = "messages";
const AUTHOR: &str = "author";
const TEXT: &str = "text";
const TS: &str = "ts";
const MSG_ID: &str = "id";
const EDITED: &str = "edited";
const REPLY_TO: &str = "reply_to";
const PINNED: &str = "pinned";
const TOPIC: &str = "topic";
const JUKEBOX: &str = "jukebox";
const JB_CID: &str = "cid";
const JB_NAME: &str = "name";
const JB_AUTHOR: &str = "author";
const JB_ADDED: &str = "added_ms";

/// Maximum length of a channel topic, in UTF-8 bytes. The topic lives in the channel
/// document, so this bounds what every member replicates; the same reason the livery and
/// avatar values are capped.
pub const MAX_CHANNEL_TOPIC_BYTES: usize = 256;

/// Maximum length of a jukebox entry's content address, in hex digits. Wide enough for any
/// address the file path mints; a value that is not hex within this budget is not a file this
/// server can ever hand back, so it is refused at write time rather than stored to fail later.
pub const MAX_JUKEBOX_CID_CHARS: usize = 128;
/// Maximum length of a jukebox entry's display name, in UTF-8 bytes; the same reason the
/// channel topic is capped, since the queue rides the channel document every member replicates.
pub const MAX_JUKEBOX_NAME_BYTES: usize = 200;
/// Maximum number of entries one channel's jukebox holds. The whole queue is replicated with
/// every change, so this bounds what a full playlist costs each member.
pub const MAX_JUKEBOX_ENTRIES: usize = 64;

/// Append a `{id, author, text, ts}` message to a channel document (the canonical edit).
pub fn append_message(
    doc: &mut AutoCommit,
    id: &str,
    author: &str,
    text: &str,
    ts: u64,
    reply_to: &str,
) -> Result<(), AutomergeError> {
    let list = match doc.get(ROOT, MESSAGES)? {
        Some((Value::Object(ObjType::List), id)) => id,
        _ => doc.put_object(ROOT, MESSAGES, ObjType::List)?,
    };
    let index = doc.length(&list);
    let msg = doc.insert_object(&list, index, ObjType::Map)?;
    doc.put(&msg, MSG_ID, id)?;
    doc.put(&msg, AUTHOR, author)?;
    doc.put(&msg, TEXT, text)?;
    doc.put(&msg, TS, ts as i64)?;
    // Only carry a reply pointer when it's actually a reply (keeps plain messages key-clean).
    if !reply_to.is_empty() {
        doc.put(&msg, REPLY_TO, reply_to)?;
    }
    Ok(())
}

/// Unread state over a whole message list, by the desktop's rule: the ceiling is the newest
/// timestamp no further ahead of `now` than the clock grace, every timestamp is clamped to it,
/// and a row counts when its clamped timestamp is past the divider and somebody else wrote it.
fn unread_summary(messages: &[ChatMessage], me: &str, probe: &UnreadProbe) -> UnreadSummary {
    let limit = probe.now_ms.saturating_add(CLOCK_SKEW_GRACE_MS);
    let ceiling_ts = messages
        .iter()
        .map(|m| m.ts)
        .filter(|ts| *ts <= limit)
        .max()
        .unwrap_or(0);
    let mut summary = UnreadSummary {
        ceiling_ts,
        ..UnreadSummary::default()
    };
    for (index, m) in messages.iter().enumerate() {
        if m.author != me && m.ts.min(ceiling_ts) > probe.divider_ts {
            summary.count += 1;
            summary.first_index.get_or_insert(index as u64);
        }
    }
    summary
}

/// Materialize a channel document into the UI's ordered message list.
///
/// One sequential cursor over the list rather than an indexed `get` per element: an indexed
/// lookup re-descends the op tree every time, which made this walk cost tens of microseconds per
/// message and put a long channel's every read (and the actor's every change check) near a
/// second. The cursor visits each element once.
pub fn read_messages(doc: &AutoCommit) -> Vec<ChatMessage> {
    use automerge::{ReadDoc as _, ValueRef};
    let mut out = Vec::new();
    if let Ok(Some((Value::Object(ObjType::List), list))) = doc.get(ROOT, MESSAGES) {
        for item in doc.list_range(&list, ..) {
            if matches!(item.value, ValueRef::Object(ObjType::Map)) {
                out.push(read_message(doc, &item.id()));
            }
        }
    }
    out
}

/// Decode one message map. Both the legacy channel/status list and the conflict-free status
/// layout below use exactly this inner schema; keeping one decoder prevents compatibility fixes
/// from making the two representations render differently.
///
/// One pass over the map's keys serves every field and the flat reaction keys alike (the same
/// cursor-not-lookup reasoning as [`read_messages`]). Field semantics are exactly those of
/// [`str_field`] / [`int_field`]: a string field that is not a string reads as empty, an integer
/// field that is not an `Int`/`Uint` reads as `0`, and `pinned` is "the key is present".
fn read_message(doc: &AutoCommit, msg: &ObjId) -> ChatMessage {
    use automerge::{ReadDoc as _, ScalarValueRef, ValueRef};
    fn str_of(value: &ValueRef<'_>) -> String {
        match value {
            ValueRef::Scalar(ScalarValueRef::Str(s)) => s.to_string(),
            _ => String::new(),
        }
    }
    fn int_of(value: &ValueRef<'_>) -> u64 {
        match value {
            ValueRef::Scalar(ScalarValueRef::Int(i)) => *i as u64,
            ValueRef::Scalar(ScalarValueRef::Uint(u)) => *u,
            _ => 0,
        }
    }
    let mut m = ChatMessage {
        id: String::new(),
        author: String::new(),
        text: String::new(),
        ts: 0,
        edited: 0,
        reactions: Vec::new(),
        reply_to: String::new(),
        pinned: false,
    };
    // Reactions are flat scalar keys *directly on the message map*, `"<emoji>\x1f<fingerprint>" =
    // true` (see `toggle_reaction_in_doc`), beside the regular field keys (none of which contain
    // the separator). A `BTreeMap` gives a stable emoji order, so the UI and the change-detector
    // signature are deterministic. No reaction keys, no reactions.
    let mut grouped: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for item in doc.map_range(msg, ..) {
        match item.key.as_ref() {
            MSG_ID => m.id = str_of(&item.value),
            AUTHOR => m.author = str_of(&item.value),
            TEXT => m.text = str_of(&item.value),
            REPLY_TO => m.reply_to = str_of(&item.value),
            TS => m.ts = int_of(&item.value),
            EDITED => m.edited = int_of(&item.value),
            PINNED => m.pinned = true,
            key => {
                if let Some((emoji, fp)) = key.split_once(REACTION_SEP) {
                    grouped
                        .entry(emoji.to_string())
                        .or_default()
                        .push(fp.to_string());
                }
            }
        }
    }
    m.reactions = grouped
        .into_iter()
        .map(|(emoji, by)| Reaction { emoji, by })
        .collect();
    m
}

/// The separator between the emoji and the reactor fingerprint in a flat reaction key. ASCII Unit
/// Separator (0x1F); a control char that appears in neither emoji nor hex fingerprints.
const REACTION_SEP: char = '\u{1f}';

/// Whether `emoji` is one this server will store as a reaction key. An emoji carrying the key
/// separator would forge a second field on the message map, so the flat-key layout only holds if
/// every document that uses it applies the same rule; hence one predicate rather than a copy per
/// feed. The length bound is a gossip budget: honest clients send a small fixed set.
fn valid_reaction_emoji(emoji: &str) -> bool {
    !emoji.is_empty() && !emoji.contains(REACTION_SEP) && emoji.len() <= 64
}

/// Toggle `fp`'s reaction `emoji` on the message with `id`: adds it if absent, removes it if
/// present. Each (emoji, reactor) pair is one flat scalar key `"<emoji>\x1f<fp>"` written *directly
/// on the message map*; which always exists. So concurrent reactors write **distinct** keys that all
/// survive a merge, and there is no sub-object that two reactors could create twice and lose one of;
/// the convergence holds for every message, including ones authored by clients predating reactions.
/// Returns whether the message was found. (`emoji` is validated by the caller; see `toggle_reaction`.)
fn toggle_reaction_in_doc(
    doc: &mut AutoCommit,
    id: &str,
    emoji: &str,
    fp: &str,
) -> Result<bool, AutomergeError> {
    let Some((Value::Object(ObjType::List), list)) = doc.get(ROOT, MESSAGES)? else {
        return Ok(false);
    };
    for i in 0..doc.length(&list) {
        let Some((Value::Object(ObjType::Map), msg)) = doc.get(&list, i)? else {
            continue;
        };
        if str_field(doc, &msg, MSG_ID) != id {
            continue;
        }
        let key = format!("{emoji}{REACTION_SEP}{fp}");
        if doc.get(&msg, &key)?.is_some() {
            doc.delete(&msg, &key)?;
        } else {
            doc.put(&msg, &key, true)?;
        }
        return Ok(true);
    }
    Ok(false)
}

/// Pin or unpin the message with `id` by setting/removing a `pinned` flag **directly on its message
/// map** (which always exists); so concurrent pins of different messages never conflict and a
/// pin/unpin race on one message is a clean last-writer-wins. Returns whether the message was found.
fn set_pin_in_doc(doc: &mut AutoCommit, id: &str, pinned: bool) -> Result<bool, AutomergeError> {
    let Some((Value::Object(ObjType::List), list)) = doc.get(ROOT, MESSAGES)? else {
        return Ok(false);
    };
    for i in 0..doc.length(&list) {
        let Some((Value::Object(ObjType::Map), msg)) = doc.get(&list, i)? else {
            continue;
        };
        if str_field(doc, &msg, MSG_ID) != id {
            continue;
        }
        if pinned {
            doc.put(&msg, PINNED, true)?;
        } else {
            doc.delete(&msg, PINNED)?;
        }
        return Ok(true);
    }
    Ok(false)
}

/// Read a channel document's topic (empty when it was never set; every channel written before
/// topics existed simply has no `topic` key, which reads as "no topic").
fn read_topic(doc: &AutoCommit) -> String {
    str_field(doc, &ROOT, TOPIC)
}

/// Set a channel document's topic: one scalar key at the document ROOT (which always exists), so
///; like `pinned` on a message map; there is no container two members could concurrently create
/// and lose one of, and a concurrent set is a clean last-writer-wins.
fn set_topic_in_doc(doc: &mut AutoCommit, topic: &str) -> Result<(), AutomergeError> {
    doc.put(ROOT, TOPIC, topic)
}

/// One entry in a channel's jukebox queue as the UI sees it. `cid` is the hex content address of
/// an already-shared file, fetched over the file path like any other embed, so an entry whose
/// blob has not arrived yet still lists fine; it just cannot play yet. The `author` is the
/// adder's **device fingerprint**, resolved to a display name at render time like a message
/// author.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JukeEntry {
    /// A stable per-entry id (random hex), minted like a message id; so a removal addresses
    /// exactly one entry under concurrent merges.
    pub id: String,
    /// The lowercase hex content address of the queued file.
    pub cid: String,
    /// The display name shown in the queue (never empty in a stored entry).
    pub name: String,
    /// The adder's device fingerprint.
    pub author: String,
    /// When the entry was queued, epoch-millis (the adder's injected clock).
    pub added_ms: u64,
}

/// Add one jukebox entry to a channel document, keyed by its id. The queue is a map at the
/// document ROOT (which always exists) and each entry is its own sub-map under a unique random
/// key, so two members queueing at once write disjoint keys that both survive a merge.
fn add_juke_entry_in_doc(doc: &mut AutoCommit, e: &JukeEntry) -> Result<(), AutomergeError> {
    let queue = match doc.get(ROOT, JUKEBOX)? {
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(ROOT, JUKEBOX, ObjType::Map)?,
    };
    let entry = doc.put_object(&queue, e.id.as_str(), ObjType::Map)?;
    doc.put(&entry, JB_CID, e.cid.as_str())?;
    doc.put(&entry, JB_NAME, e.name.as_str())?;
    doc.put(&entry, JB_AUTHOR, e.author.as_str())?;
    doc.put(&entry, JB_ADDED, e.added_ms as i64)?;
    Ok(())
}

/// Remove the jukebox entry with `id` from a channel document. Ids are unique keys, so this
/// removes (at most) the single matching entry. Returns whether one was there.
fn remove_juke_entry_in_doc(doc: &mut AutoCommit, id: &str) -> Result<bool, AutomergeError> {
    let Some((Value::Object(ObjType::Map), queue)) = doc.get(ROOT, JUKEBOX)? else {
        return Ok(false);
    };
    if doc.get(&queue, id)?.is_none() {
        return Ok(false);
    }
    doc.delete(&queue, id)?;
    Ok(true)
}

/// Materialize a channel document's jukebox, sorted by **queue time ascending** (ties broken by
/// id, so every member reads the same order). An entry without a usable content address or name
/// is skipped, so a malformed doc degrades to "fewer tracks" rather than unplayable rows.
fn read_jukebox(doc: &AutoCommit) -> Vec<JukeEntry> {
    let mut out = Vec::new();
    if let Ok(Some((Value::Object(ObjType::Map), queue))) = doc.get(ROOT, JUKEBOX) {
        for id in doc.keys(&queue) {
            if let Ok(Some((Value::Object(ObjType::Map), entry))) = doc.get(&queue, &id) {
                let cid = juke_cid_field(doc, &entry, JB_CID);
                let name = str_field(doc, &entry, JB_NAME);
                if cid.is_empty() || name.is_empty() {
                    continue; // a cleared/malformed entry is not a playable track
                }
                out.push(JukeEntry {
                    id,
                    cid,
                    name,
                    author: str_field(doc, &entry, JB_AUTHOR),
                    added_ms: int_field(doc, &entry, JB_ADDED),
                });
            }
        }
    }
    out.sort_by(|a, b| a.added_ms.cmp(&b.added_ms).then_with(|| a.id.cmp(&b.id)));
    out
}

/// Read a jukebox entry's content address, applying the same shape check the writer applies
/// (see [`MAX_JUKEBOX_CID_CHARS`]); a peer that wrote junk reads as "no address" rather than as
/// something the UI would try to fetch.
fn juke_cid_field(doc: &AutoCommit, obj: &ObjId, key: &str) -> String {
    let v = str_field(doc, obj, key);
    if valid_juke_cid(&v) {
        v
    } else {
        String::new()
    }
}

/// Whether `cid` is a plausible file content address: 1..=[`MAX_JUKEBOX_CID_CHARS`] lowercase
/// hex digits. Deliberately strict about case, so one file has exactly one queue-visible
/// spelling and a duplicate cannot hide behind a different one.
fn valid_juke_cid(cid: &str) -> bool {
    !cid.is_empty()
        && cid.len() <= MAX_JUKEBOX_CID_CHARS
        && cid.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Edit the text of the message with `id` in a channel document, stamping `edited`. Returns
/// whether a message was found + changed. (Honest-client own-message gating is the caller's job.)
fn edit_message_in_doc(
    doc: &mut AutoCommit,
    id: &str,
    new_text: &str,
    edited_ts: u64,
) -> Result<bool, AutomergeError> {
    let Some((Value::Object(ObjType::List), list)) = doc.get(ROOT, MESSAGES)? else {
        return Ok(false);
    };
    for i in 0..doc.length(&list) {
        if let Some((Value::Object(ObjType::Map), msg)) = doc.get(&list, i)? {
            if str_field(doc, &msg, MSG_ID) == id {
                doc.put(&msg, TEXT, new_text)?;
                doc.put(&msg, EDITED, edited_ts as i64)?;
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Delete the message with `id` from a channel document. Returns whether one was removed. Ids are
/// unique, so it removes (at most) the single matching element and returns.
fn delete_message_in_doc(doc: &mut AutoCommit, id: &str) -> Result<bool, AutomergeError> {
    let Some((Value::Object(ObjType::List), list)) = doc.get(ROOT, MESSAGES)? else {
        return Ok(false);
    };
    for i in 0..doc.length(&list) {
        if let Some((Value::Object(ObjType::Map), msg)) = doc.get(&list, i)? {
            if str_field(doc, &msg, MSG_ID) == id {
                doc.delete(&list, i)?;
                return Ok(true);
            }
        }
    }
    Ok(false)
}

// --- the status feed's conflict-free post layout ----------------------------
//
// Early status feeds reused the channel `messages` list. If two peers made the first post before
// either had received the other's list-creation op, Automerge retained both conflicting lists but
// `get(ROOT, MESSAGES)` could expose only one of them. One peer's posts then appeared to vanish.
// New status posts are child maps at distinct ROOT keys derived from their random ids. Concurrent
// authors therefore edit disjoint keys and all posts survive. Readers and mutators retain the old
// list path so existing documents remain fully usable; this is an additive persistence change.

const STATUS_POST_PREFIX: &str = "status_post/";
const STATUS_POST_ID_HEX_CHARS: usize = 32;

fn status_post_key(id: &str) -> String {
    format!("{STATUS_POST_PREFIX}{id}")
}

fn valid_status_post_id(id: &str) -> bool {
    id.len() == STATUS_POST_ID_HEX_CHARS
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[derive(Clone)]
enum StatusPostLocation {
    Keyed { key: String },
    Legacy { list: ObjId, index: usize },
}

#[derive(Clone)]
struct StatusPostCandidate {
    post: ChatMessage,
    obj: ObjId,
    location: StatusPostLocation,
}

/// Append a status post without requiring peers to agree on a shared container first.
fn append_status_message(
    doc: &mut AutoCommit,
    id: &str,
    author: &str,
    text: &str,
    ts: u64,
) -> Result<(), AutomergeError> {
    let msg = doc.put_object(ROOT, status_post_key(id), ObjType::Map)?;
    doc.put(&msg, MSG_ID, id)?;
    doc.put(&msg, AUTHOR, author)?;
    doc.put(&msg, TEXT, text)?;
    doc.put(&msg, TS, ts as i64)?;
    Ok(())
}

/// Enumerate every status candidate, including every Automerge conflict at the legacy reserved
/// list key and at keyed post properties. This is deliberately status-specific: channel messages
/// retain their canonical list behavior, while the upgrade path must recover already-conflicted
/// status lists rather than merely preventing the next conflict.
fn status_post_candidates(doc: &AutoCommit) -> Result<Vec<StatusPostCandidate>, AutomergeError> {
    let mut out = Vec::new();
    for (value, list) in doc.get_all(ROOT, MESSAGES)? {
        if !matches!(value, Value::Object(ObjType::List)) {
            continue;
        }
        for index in 0..doc.length(&list) {
            for (value, msg) in doc.get_all(&list, index)? {
                if matches!(value, Value::Object(ObjType::Map)) {
                    out.push(StatusPostCandidate {
                        post: read_message(doc, &msg),
                        obj: msg,
                        location: StatusPostLocation::Legacy {
                            list: list.clone(),
                            index,
                        },
                    });
                }
            }
        }
    }
    for key in doc.keys(ROOT) {
        if !key.starts_with(STATUS_POST_PREFIX) {
            continue;
        }
        for (value, msg) in doc.get_all(ROOT, &key)? {
            if !matches!(value, Value::Object(ObjType::Map)) {
                continue;
            }
            let post = read_message(doc, &msg);
            // Empty, non-canonical, or key-mismatched ids are inert. In particular,
            // `status_post/` + `id=""` must not create a visible row no product command can address.
            if valid_status_post_id(&post.id) && status_post_key(&post.id) == key {
                out.push(StatusPostCandidate {
                    post,
                    obj: msg,
                    location: StatusPostLocation::Keyed { key: key.clone() },
                });
            }
        }
    }
    Ok(out)
}

/// Read legacy list posts plus conflict-free keyed posts in one deterministic oldest-first feed.
/// A canonical non-empty id is rendered only when it addresses exactly one object across every
/// representation/conflict. Ambiguous rows are hidden and all mutations fail closed, preventing
/// authorization against one author followed by mutation of another object with the same id.
/// Truly old id-less legacy posts remain readable but intentionally non-actionable.
fn read_status_messages(doc: &AutoCommit) -> Vec<ChatMessage> {
    let candidates = status_post_candidates(doc).unwrap_or_default();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for candidate in &candidates {
        if valid_status_post_id(&candidate.post.id) {
            *counts.entry(candidate.post.id.clone()).or_default() += 1;
        }
    }
    let mut out: Vec<ChatMessage> = candidates
        .into_iter()
        .filter(|candidate| {
            (candidate.post.id.is_empty()
                && matches!(candidate.location, StatusPostLocation::Legacy { .. }))
                || (valid_status_post_id(&candidate.post.id)
                    && counts.get(&candidate.post.id) == Some(&1))
        })
        .map(|candidate| candidate.post)
        .collect();
    out.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.id.cmp(&b.id)));
    out
}

/// Resolve exactly one canonical post across keyed and every legacy-conflict representation.
/// `None` covers missing, malformed, and ambiguous ids; all three must be non-actionable.
fn resolve_status_message(
    doc: &AutoCommit,
    id: &str,
) -> Result<Option<StatusPostCandidate>, AutomergeError> {
    if !valid_status_post_id(id) {
        return Ok(None);
    }
    let mut matches = status_post_candidates(doc)?
        .into_iter()
        .filter(|candidate| candidate.post.id == id);
    let first = matches.next();
    if first.is_none() || matches.next().is_some() {
        return Ok(None);
    }
    Ok(first)
}

fn edit_status_message_in_doc(
    doc: &mut AutoCommit,
    id: &str,
    new_text: &str,
    edited_ts: u64,
) -> Result<bool, AutomergeError> {
    let Some(candidate) = resolve_status_message(doc, id)? else {
        return Ok(false);
    };
    doc.put(&candidate.obj, TEXT, new_text)?;
    doc.put(&candidate.obj, EDITED, edited_ts as i64)?;
    Ok(true)
}

fn delete_status_message_in_doc(doc: &mut AutoCommit, id: &str) -> Result<bool, AutomergeError> {
    let Some(candidate) = resolve_status_message(doc, id)? else {
        return Ok(false);
    };
    match candidate.location {
        StatusPostLocation::Keyed { key } => doc.delete(ROOT, key)?,
        StatusPostLocation::Legacy { list, index } => doc.delete(&list, index)?,
    }
    Ok(true)
}

fn toggle_status_reaction_in_doc(
    doc: &mut AutoCommit,
    id: &str,
    emoji: &str,
    fp: &str,
) -> Result<bool, AutomergeError> {
    let Some(candidate) = resolve_status_message(doc, id)? else {
        return Ok(false);
    };
    let key = format!("{emoji}{REACTION_SEP}{fp}");
    if doc.get(&candidate.obj, &key)?.is_some() {
        doc.delete(&candidate.obj, &key)?;
    } else {
        doc.put(&candidate.obj, &key, true)?;
    }
    Ok(true)
}

fn set_status_pin_in_doc(
    doc: &mut AutoCommit,
    id: &str,
    pinned: bool,
) -> Result<bool, AutomergeError> {
    let Some(candidate) = resolve_status_message(doc, id)? else {
        return Ok(false);
    };
    if pinned {
        doc.put(&candidate.obj, PINNED, true)?;
    } else {
        doc.delete(&candidate.obj, PINNED)?;
    }
    Ok(true)
}

fn str_field(doc: &AutoCommit, obj: &ObjId, key: &str) -> String {
    doc.get(obj, key)
        .ok()
        .flatten()
        .and_then(|(v, _)| v.into_string().ok())
        .unwrap_or_default()
}

/// Read an integer scalar field as `u64` (`0` if absent or another type).
fn int_field(doc: &AutoCommit, obj: &ObjId, key: &str) -> u64 {
    match doc.get(obj, key) {
        Ok(Some((Value::Scalar(s), _))) => match s.as_ref() {
            ScalarValue::Int(i) => *i as u64,
            ScalarValue::Uint(u) => *u,
            _ => 0,
        },
        _ => 0,
    }
}

// --- the status feed's posting policy ---------------------------------------
//
// The status feed is the schema above on its own per-server document, so every message helper
// already reaches it. What it does not inherit from a chat channel is *who may write*: a channel
// is conversation, the feed is the server announcing itself. That decision is one boolean at the
// feed document's ROOT, replicated with the feed, so there is no second document that could drift
// out of step with the posts it governs.

/// The status feed's root key holding its posting policy.
const STATUS_MAY_POST: &str = "members_may_post";

/// Whether plain members may post to this status feed. Absent (every feed written before the
/// policy existed) and anything stored under the key that is not a boolean both read as `false`:
/// a document from a peer that has never heard of the policy, or one carrying junk, closes the
/// feed rather than opening it.
fn read_status_may_post(doc: &AutoCommit) -> bool {
    match doc.get(ROOT, STATUS_MAY_POST) {
        Ok(Some((Value::Scalar(s), _))) => matches!(s.as_ref(), ScalarValue::Boolean(true)),
        _ => false,
    }
}

/// Set the posting policy: one scalar key at the ROOT (which always exists), so; like `pinned` on
/// a message map; there is no container two admins could concurrently create and lose one of, and
/// a concurrent change is a clean last-writer-wins.
fn write_status_may_post(doc: &mut AutoCommit, allow: bool) -> Result<(), AutomergeError> {
    doc.put(ROOT, STATUS_MAY_POST, allow)
}

// --- member profiles --------------------------------------------------------
//
// One shared CRDT document per server (`DocType::Profile`, id `PROFILE_DOC`), a map
// keyed by member **device fingerprint** → `{ name, color, font, effect }`. Each member
// writes only their *own* fingerprint's entry by convention (the op is inner-signed by
// the author's device; enforcing "an op may only touch the author's own key" is a later
// hardening; a malicious member overwriting another's profile is low-stakes and
// detectable, not a confidentiality/integrity break).

/// The reserved document id for the per-server profile document.
const PROFILE_DOC: u128 = 0;
const P_NAME: &str = "name";
const P_COLOR: &str = "color";
const P_FONT: &str = "font";
const P_EFFECT: &str = "effect";
const P_AVATAR_CID: &str = "avatar_cid";
const P_BANNER_CID: &str = "banner_cid";
const P_DESCRIPTION: &str = "description";
const P_BUBBLE: &str = "bubble";

/// Maximum avatar image size accepted by [`Server::set_profile`]. Avatars are stored by
/// **content address** in the blob store (not inline in the gossiped profile document)
/// and fetched on demand over the mesh; this caps the blob the UI's downscaled ~128px
/// JPEG produces.
pub const MAX_AVATAR_BYTES: usize = 64 * 1024;

/// Maximum profile banner image size accepted by [`Server::set_profile`]. A banner rides by
/// **content address** exactly like an avatar (only the CID gossips in the profile document,
/// the bytes are fetched on demand), so this budget bounds a blob fetch rather than gossip.
/// It is larger than [`MAX_AVATAR_BYTES`] because a banner is a wide profile-card image, not
/// a 128px square, so the same visual quality needs more pixels.
pub const MAX_BANNER_BYTES: usize = 256 * 1024;

/// Max avatar blobs fetched per [`Server::fetch_missing_avatars`] pass; bounds how long
/// one pass (each fetch a blocking mesh round-trip) can stall the actor, so avatar churn by
/// a malicious member cannot freeze peers' event loops.
const MAX_AVATAR_FETCHES_PER_PASS: usize = 8;

/// A member's customizable profile. `avatar` is the **resolved image bytes** (empty if
/// unset, or not yet fetched from the mesh); on the wire the profile document carries only
/// the avatar's content address.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    /// Chosen display name.
    pub name: String,
    /// Name/text color (a CSS color string, e.g. `#4f8cff`).
    pub color: String,
    /// A UI font key (e.g. `system` | `serif` | `mono`).
    pub font: String,
    /// A text-effect key (e.g. `none` | `rainbow` | `wave` | `pulse`).
    pub effect: String,
    /// A short self-description / bio (shown on the profile card).
    pub description: String,
    /// A CSS background for this member's message bubble (a color or gradient); empty = default.
    pub bubble: String,
    /// The avatar image bytes, resolved from its content address against the local blob
    /// store (empty if unset or not yet fetched). The UI produces a downscaled JPEG.
    pub avatar: Vec<u8>,
    /// The profile banner image bytes, resolved from its content address the same way as
    /// [`Profile::avatar`] (empty if unset or not yet fetched). A wide profile-card image.
    pub banner: Vec<u8>,
}

/// An internal profile record straight from the document: the avatar and banner are their
/// **content addresses** (CID bytes), resolved to image bytes against the blob store by
/// [`Server`].
struct ProfileRecord {
    name: String,
    color: String,
    font: String,
    effect: String,
    description: String,
    bubble: String,
    avatar_cid: Vec<u8>,
    banner_cid: Vec<u8>,
}

/// Write a member's own profile entry. The avatar and banner are referenced by **content
/// address** (`avatar_cid` / `banner_cid`), not stored inline; so the gossiped profile
/// document stays tiny and the images are fetched on demand over the mesh.
fn write_profile(
    doc: &mut AutoCommit,
    fp: &str,
    p: &Profile,
    avatar_cid: &[u8],
    banner_cid: &[u8],
) -> Result<(), AutomergeError> {
    let entry = match doc.get(ROOT, fp)? {
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(ROOT, fp, ObjType::Map)?,
    };
    doc.put(&entry, P_NAME, p.name.as_str())?;
    doc.put(&entry, P_COLOR, p.color.as_str())?;
    doc.put(&entry, P_FONT, p.font.as_str())?;
    doc.put(&entry, P_EFFECT, p.effect.as_str())?;
    doc.put(&entry, P_DESCRIPTION, p.description.as_str())?;
    doc.put(&entry, P_BUBBLE, p.bubble.as_str())?;
    doc.put(
        &entry,
        P_AVATAR_CID,
        ScalarValue::Bytes(avatar_cid.to_vec()),
    )?;
    doc.put(
        &entry,
        P_BANNER_CID,
        ScalarValue::Bytes(banner_cid.to_vec()),
    )?;
    Ok(())
}

/// Materialize the profile document into `fingerprint -> ProfileRecord` (avatars and banners
/// still as content addresses; [`Server::profiles`] resolves them against the blob store).
fn read_profile_records(doc: &AutoCommit) -> HashMap<String, ProfileRecord> {
    let mut out = HashMap::new();
    for fp in doc.keys(ROOT) {
        if let Ok(Some((Value::Object(ObjType::Map), entry))) = doc.get(ROOT, &fp) {
            out.insert(
                fp,
                ProfileRecord {
                    name: str_field(doc, &entry, P_NAME),
                    color: str_field(doc, &entry, P_COLOR),
                    font: str_field(doc, &entry, P_FONT),
                    effect: str_field(doc, &entry, P_EFFECT),
                    description: str_field(doc, &entry, P_DESCRIPTION),
                    bubble: str_field(doc, &entry, P_BUBBLE),
                    avatar_cid: bytes_field(doc, &entry, P_AVATAR_CID),
                    banner_cid: bytes_field(doc, &entry, P_BANNER_CID),
                },
            );
        }
    }
    out
}

/// Parse a stored avatar/banner content address (32 bytes) into a [`Cid`] (`None` if
/// absent/bad).
fn parse_avatar_cid(bytes: &[u8]) -> Option<Cid> {
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some(Cid::from_bytes(arr))
}

// --- server livery ----------------------------------------------------------
//
// One shared CRDT document per server (`DocType::Livery`, id `LIVERY_DOC`): the
// owner/admin-published UI colour scheme every member's client applies while that server is
// active (`docs/design-livery.md`). Written by owners/admins only; honest-client gating at
// the same policy layer as roles/pins (the op log is inner-signed, so authorship of a
// forged write is attributable either way).
//
// The values are stored **opaquely**: the backend bounds their sizes, the *client* validates
// them on read (known preset ids, `#rrggbb` accents, an allow-list of colour tokens) and
// degrades a malformed doc to "no livery". Nothing here can reach layout or fetch a URL.

/// The reserved document id for the per-server livery document.
const LIVERY_DOC: u128 = 0;
/// Schema version written into every livery doc, so a later shape can be told apart.
const LIVERY_VERSION: i64 = 1;
const L_V: &str = "v";
const L_PRESET: &str = "preset";
const L_ACCENT: &str = "accent";
const L_TOKENS: &str = "tokens";
/// The shared server icon (base64 image bytes). Additive to the v1 schema: an older doc
/// simply lacks the key, which reads as "no icon".
const L_ICON: &str = "icon";
/// The shared server cursor (base64 image bytes). Additive to the v1 schema in the same way
/// as the icon: an older doc simply lacks the key, which reads as "no cursor".
const L_CURSOR: &str = "cursor";

/// Maximum length of a livery preset id (a short key like `nightshade`).
pub const MAX_LIVERY_PRESET_BYTES: usize = 32;
/// Maximum length of a livery accent value (`#rrggbb` plus slack).
pub const MAX_LIVERY_ACCENT_BYTES: usize = 16;
/// Maximum number of colour-token overrides a livery may carry.
pub const MAX_LIVERY_TOKENS: usize = 16;
/// Maximum length of a livery token name (e.g. `--accent-hi`).
pub const MAX_LIVERY_TOKEN_KEY_BYTES: usize = 32;
/// Maximum length of a livery token value (`#rrggbb` plus slack).
pub const MAX_LIVERY_TOKEN_VALUE_BYTES: usize = 16;
/// Maximum **decoded** size of a server icon accepted by [`Server::set_server_icon`]; the
/// same budget as a member avatar ([`MAX_AVATAR_BYTES`]), since the UI produces the same
/// kind of small downscaled image. Unlike an avatar the icon rides *inline* (base64) in the
/// livery document rather than by content address, so this cap also bounds what gossips.
pub const MAX_SERVER_ICON_BYTES: usize = 64 * 1024;
/// Maximum **decoded** size of a server cursor accepted by [`Server::set_server_cursor`]. A
/// cursor image is at most 64×64 (a hotspot-bearing pointer, not artwork), so it gets a far
/// tighter budget than the icon; and, like the icon, it rides *inline* (base64) in the livery
/// document, so this also bounds what gossips.
pub const MAX_SERVER_CURSOR_BYTES: usize = 16 * 1024;

/// A server's published UI livery. Empty fields mean "no livery" / "no override"; every
/// value is opaque to the backend and validated by the client on read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Livery {
    /// A preset id (e.g. `nightshade`); empty = the client's own scheme.
    pub preset: String,
    /// An accent colour override (`#rrggbb`); empty = the preset's own accent.
    pub accent: String,
    /// Bounded colour-token overrides (token name -> colour); empty in v1 liveries.
    pub tokens: HashMap<String, String>,
    /// The shared server icon as base64 image bytes; empty = no icon. Set/cleared only by
    /// [`Server::set_server_icon`]; [`Server::set_livery`] ignores this field and preserves
    /// whatever is stored, so publishing colours never resends or drops the image.
    pub icon: String,
    /// The shared server cursor as base64 image bytes; empty = no cursor. Set/cleared only by
    /// [`Server::set_server_cursor`], exactly like the icon: [`Server::set_livery`] ignores
    /// this field and preserves whatever is stored. The two images are independent; setting
    /// one never disturbs the other.
    pub cursor: String,
}

/// Write the livery document (last-writer-wins on each field; the token map is replaced
/// wholesale so removing an override actually removes it). Writes **every** field, the icon
/// and cursor included, so callers that must not disturb the stored images read them back
/// into `l.icon`/`l.cursor` first (see [`Server::set_livery`]).
fn write_livery(doc: &mut AutoCommit, l: &Livery) -> Result<(), AutomergeError> {
    doc.put(ROOT, L_V, LIVERY_VERSION)?;
    doc.put(ROOT, L_PRESET, l.preset.as_str())?;
    doc.put(ROOT, L_ACCENT, l.accent.as_str())?;
    doc.put(ROOT, L_ICON, l.icon.as_str())?;
    doc.put(ROOT, L_CURSOR, l.cursor.as_str())?;
    let tokens = doc.put_object(ROOT, L_TOKENS, ObjType::Map)?;
    for (k, v) in &l.tokens {
        doc.put(&tokens, k.as_str(), v.as_str())?;
    }
    Ok(())
}

/// Write **only** the server icon (`""` clears it), leaving the colour fields untouched;
/// the image is a separate, much larger value with its own command, so the two never have
/// to be republished together.
fn write_server_icon(doc: &mut AutoCommit, icon: &str) -> Result<(), AutomergeError> {
    doc.put(ROOT, L_V, LIVERY_VERSION)?;
    doc.put(ROOT, L_ICON, icon)?;
    Ok(())
}

/// Write **only** the server cursor (`""` clears it), leaving the colour fields *and the icon*
/// untouched; the mirror of [`write_server_icon`], so the two images have wholly independent
/// lifetimes and neither has to be republished with the other.
fn write_server_cursor(doc: &mut AutoCommit, cursor: &str) -> Result<(), AutomergeError> {
    doc.put(ROOT, L_V, LIVERY_VERSION)?;
    doc.put(ROOT, L_CURSOR, cursor)?;
    Ok(())
}

/// Materialize the livery document (a missing/foreign-shaped field reads as empty; so a
/// doc written before the icon/cursor keys existed reads back with neither).
fn read_livery(doc: &AutoCommit) -> Livery {
    let mut tokens = HashMap::new();
    if let Ok(Some((Value::Object(ObjType::Map), map))) = doc.get(ROOT, L_TOKENS) {
        for k in doc.keys(&map) {
            let v = str_field(doc, &map, &k);
            tokens.insert(k, v);
        }
    }
    Livery {
        preset: str_field(doc, &ROOT, L_PRESET),
        accent: str_field(doc, &ROOT, L_ACCENT),
        tokens,
        icon: str_field(doc, &ROOT, L_ICON),
        cursor: str_field(doc, &ROOT, L_CURSOR),
    }
}

// --- custom member badges ----------------------------------------------------
//
// One shared CRDT document per server (`DocType::Badges`, id `BADGES_DOC`): a map keyed by
// member **device fingerprint** → `{ label, color }`, the small labelled tag an owner/admin
// pins next to a member's name (e.g. `ARTIST` in teal). Written by owners/admins only;
// honest-client gating at the same policy layer as the livery/roles/pins (the op log is
// inner-signed, so authorship of a forged write is attributable either way).
//
// Like the livery, the values are stored **opaquely**: the backend bounds their sizes and
// count and rejects labels reserved for the built-in roles; the *client* validates the colour
// on read (and likewise ignores a reserved label, in case one predates this gate).

/// The reserved document id for the per-server badge document.
const BADGES_DOC: u128 = 0;
/// Schema version written into every badge doc, so a later shape can be told apart.
const BADGES_VERSION: i64 = 1;
const B_V: &str = "v";
/// The map of `fingerprint -> { label, color }` inside the badge document. Kept under its own
/// key (rather than at the root, like the profile doc) so the schema version can live beside it
/// without ever colliding with a fingerprint.
const B_BADGES: &str = "badges";
const B_LABEL: &str = "label";
const B_COLOR: &str = "color";

/// Maximum length of a badge label (a short word like `ARTIST`).
pub const MAX_BADGE_LABEL_BYTES: usize = 24;
/// Maximum length of a badge colour value (`#rrggbb` plus slack).
pub const MAX_BADGE_COLOR_BYTES: usize = 16;
/// Maximum length of the fingerprint a badge is keyed by (a hex device fingerprint).
pub const MAX_BADGE_FINGERPRINT_BYTES: usize = 128;
/// Maximum number of badge entries one server's document may carry; the whole map gossips
/// with every change, so this bounds what members replicate.
pub const MAX_BADGES: usize = 128;

/// Labels reserved for the built-in roles: a custom badge may not impersonate one (compared
/// case-insensitively, after trimming). The client applies the same rule on read.
pub const RESERVED_BADGE_LABELS: [&str; 4] = ["owner", "admin", "mod", "moderator"];

/// A member's custom badge; a short label plus a colour, both opaque to the backend.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberBadge {
    /// The badge text (e.g. `ARTIST`); never empty in a stored entry (an empty label removes it).
    pub label: String,
    /// The badge colour (a CSS colour string, e.g. `#3fb8af`); empty = the client's default.
    pub color: String,
}

/// Write (or, with an empty `label`, remove) one member's badge entry. Last-writer-wins per
/// member, so two admins badging *different* members never conflict.
fn write_member_badge(
    doc: &mut AutoCommit,
    fp: &str,
    badge: &MemberBadge,
) -> Result<(), AutomergeError> {
    doc.put(ROOT, B_V, BADGES_VERSION)?;
    let badges = match doc.get(ROOT, B_BADGES)? {
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(ROOT, B_BADGES, ObjType::Map)?,
    };
    if badge.label.is_empty() {
        doc.delete(&badges, fp)?;
        return Ok(());
    }
    let entry = match doc.get(&badges, fp)? {
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(&badges, fp, ObjType::Map)?,
    };
    doc.put(&entry, B_LABEL, badge.label.as_str())?;
    doc.put(&entry, B_COLOR, badge.color.as_str())?;
    Ok(())
}

/// Materialize the badge document into `fingerprint -> MemberBadge` (a missing/foreign-shaped
/// entry is skipped, so a malformed doc degrades to "no badges" rather than junk labels).
fn read_badges(doc: &AutoCommit) -> HashMap<String, MemberBadge> {
    let mut out = HashMap::new();
    if let Ok(Some((Value::Object(ObjType::Map), badges))) = doc.get(ROOT, B_BADGES) {
        for fp in doc.keys(&badges) {
            if let Ok(Some((Value::Object(ObjType::Map), entry))) = doc.get(&badges, &fp) {
                let label = str_field(doc, &entry, B_LABEL);
                if label.is_empty() {
                    continue; // a cleared/empty entry is "no badge"
                }
                out.insert(
                    fp,
                    MemberBadge {
                        label,
                        color: str_field(doc, &entry, B_COLOR),
                    },
                );
            }
        }
    }
    out
}

/// Whether `label` (trimmed, case-insensitive) is one of the role-reserved words.
fn is_reserved_badge_label(label: &str) -> bool {
    let label = label.trim();
    RESERVED_BADGE_LABELS
        .iter()
        .any(|r| label.eq_ignore_ascii_case(r))
}

// --- companion devices (multi-device M3) -------------------------------------
//
// One shared CRDT document per server (`DocType::Devices`, id `DEVICES_DOC`): a map keyed by
// **companion device fingerprint** → `{ origin fingerprint, device name, certificate }`. It is
// what lets any member attribute a companion's ops to the member's origin identity, which is how
// profiles/roles/badges stay origin-keyed with no doc re-keying at all
// (`docs/design-multi-device.md`).
//
// The **owner writes it, at admission time**. Admission is owner-serialized; only the designated
// committer runs the MLS Add; so unlike every other shared document here there is exactly one
// writer and no write race to reason about.
//
// Every entry is re-derived from its stored certificate on read: it survives only if the
// certificate verifies under the origin it names, is bound to THIS group, and its subject, origin
// and name match the key and the fields stored beside it. A modified client that writes a bogus
// entry therefore changes nothing; forging one would take an origin's private key. That matters
// more here than for the livery or badges, because the owner's **depth-1 admission gate** reads
// this map: without the re-derivation, a member could nominate someone as a "companion" and stop
// them ever certifying a device of their own.

/// The reserved document id for the per-server companion-device registry.
const DEVICES_DOC: u128 = 0;
/// Schema version written into every devices document.
const DEVICES_VERSION: i64 = 1;
const D_V: &str = "v";
/// The map of `companion fingerprint -> { origin, name, cert }`, under its own key so the schema
/// version can live beside it without ever colliding with a fingerprint.
const D_DEVICES: &str = "devices";
/// **Reserved for M5 (revocation).** A sibling map of `revoked fingerprint -> revocation bytes`,
/// which an origin writes to withdraw one of its own companions. Nothing writes it today; the
/// reader ([`read_device_revocations`]) and the owner's admission refusal are already in place, so
/// M5 is a write path plus the MLS Remove, with no change to the checks.
const D_REVOCATIONS: &str = "revocations";
const D_ORIGIN: &str = "origin";
const D_NAME: &str = "name";
const D_CERT: &str = "cert";
/// The **owner's** signing public key (32 bytes) stored beside each entry.
const D_OWNER_PK: &str = "opk";
/// The **owner's** signature over the entry (see [`device_entry_payload`]). A valid device
/// certificate only proves an origin *wanted* this device; the owner's signature proves the
/// owner *admitted* it. Without this, any member could `post` a self-minted (genuinely signed)
/// certificate naming an arbitrary subject straight into the doc; poisoning the depth-1 gate,
/// spoofing attribution, or marking the owner a companion. So the reader requires it, exactly as
/// the member-roles roster requires the owner's signature (adversarial-review BLOCKING finding).
const D_OWNER_SIG: &str = "osig";

/// Domain separator for the owner's per-entry devices-registry signature.
const DEVICES_DOMAIN: &[u8] = b"catcoms/devices-registry/v1";

/// The canonical bytes the owner signs to attest one admitted companion entry: domain, then the
/// group id, companion fingerprint, and full certificate bytes, each length-prefixed so no field
/// boundary can be shifted. Binding the group means an entry signed for one server can't be
/// replayed into another's registry.
fn device_entry_payload(group_id: &[u8], companion_fp: &str, cert_bytes: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(DEVICES_DOMAIN.len() + 12 + group_id.len() + cert_bytes.len());
    p.extend_from_slice(DEVICES_DOMAIN);
    p.extend_from_slice(&(group_id.len() as u32).to_be_bytes());
    p.extend_from_slice(group_id);
    p.extend_from_slice(&(companion_fp.len() as u32).to_be_bytes());
    p.extend_from_slice(companion_fp.as_bytes());
    p.extend_from_slice(&(cert_bytes.len() as u32).to_be_bytes());
    p.extend_from_slice(cert_bytes);
    p
}

/// Maximum number of companion devices one server's registry may carry; the whole map gossips
/// with every change, so this bounds what members replicate. Generous next to [`MAX_BADGES`]:
/// every member may have several devices.
pub const MAX_DEVICES: usize = 256;

/// One companion device as the product layer sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEntry {
    /// The **origin** device's fingerprint; the member identity this device's ops belong to, and
    /// the key its profile / roles / badges are stored under.
    pub origin: String,
    /// The human-set device name the origin certified (e.g. `phone`), rendered as the mono device
    /// tag beside a message author at M4. Already bounded and control-character-free: it is
    /// inside the certificate's signature and [`catcoms_crypto::validate_device_name`] gates it.
    pub name: String,
}

/// Record one admitted companion. Called by the **owner only**, from the admission drain.
///
/// `owner_pk` is the owner's signing key and `owner_sig` its signature over
/// [`device_entry_payload`] for this entry; the reader requires both, so an entry a non-owner
/// forges into the doc (a genuinely-signed certificate for an arbitrary subject) is dropped.
fn write_device_entry(
    doc: &mut AutoCommit,
    cert: &DeviceCertificate,
    owner_pk: &[u8],
    owner_sig: &[u8; 64],
) -> Result<(), AutomergeError> {
    doc.put(ROOT, D_V, DEVICES_VERSION)?;
    let devices = match doc.get(ROOT, D_DEVICES)? {
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(ROOT, D_DEVICES, ObjType::Map)?,
    };
    let fp = fingerprint(&cert.new_device_id);
    let entry = match doc.get(&devices, &fp)? {
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(&devices, &fp, ObjType::Map)?,
    };
    doc.put(&entry, D_ORIGIN, fingerprint(&cert.origin_id).as_str())?;
    doc.put(&entry, D_NAME, cert.device_name.as_str())?;
    doc.put(&entry, D_CERT, ScalarValue::Bytes(cert.encode()))?;
    doc.put(&entry, D_OWNER_PK, ScalarValue::Bytes(owner_pk.to_vec()))?;
    doc.put(&entry, D_OWNER_SIG, ScalarValue::Bytes(owner_sig.to_vec()))?;
    Ok(())
}

/// The set of companion devices a batch of revocations actually revokes: a revocation counts
/// only when its origin matches the companion's **registered** origin (`companions`). This is the
/// cross-check that stops member A from evicting member B's device by signing a revocation that
/// names B's device id; A holds only its own origin key, and even a syntactically-valid
/// revocation for someone else's device is ignored. (M5; pulled out so it is unit-testable
/// without any wire/tick machinery.)
fn honored_revocations(
    companions: &HashMap<DeviceId, DeviceId>,
    revocations: &[DeviceRevocation],
) -> HashSet<DeviceId> {
    revocations
        .iter()
        .filter(|r| companions.get(&r.revoked_device_id) == Some(&r.origin_id))
        .map(|r| r.revoked_device_id)
        .collect()
}

/// Record one device revocation (M5). Written by the revoked device's **origin**; the
/// revocation is self-authenticating (origin-signed), and the reader only honours one whose origin
/// matches the companion's registered origin, so no owner counter-signature is needed here.
fn write_revocation_entry(
    doc: &mut AutoCommit,
    rev: &DeviceRevocation,
) -> Result<(), AutomergeError> {
    doc.put(ROOT, D_V, DEVICES_VERSION)?;
    let revs = match doc.get(ROOT, D_REVOCATIONS)? {
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(ROOT, D_REVOCATIONS, ObjType::Map)?,
    };
    let fp = fingerprint(&rev.revoked_device_id);
    let entry = match doc.get(&revs, &fp)? {
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(&revs, &fp, ObjType::Map)?,
    };
    doc.put(&entry, D_CERT, ScalarValue::Bytes(rev.encode()))?;
    Ok(())
}

/// Every companion certificate in the registry that is **genuine for this group**.
///
/// An entry is kept only if its certificate decodes, verifies under the origin it names, is bound
/// to `group_id`, and agrees with the key and sibling fields it was stored with. Anything else is
/// skipped, so a malformed or forged document degrades to "no companions" rather than to a wrong
/// attribution or a bogus depth-1 refusal.
///
/// **The owner's signature is what makes an entry trustworthy.** A device certificate only
/// proves that some origin device *wanted* this companion; it does not prove the group *admitted*
/// it, and its subject can be any device id. So every entry must additionally carry the current
/// owner's signature over [`device_entry_payload`] (`owner_id` is the group's designated
/// committer), and an entry missing or failing that check is dropped; closing the forged-entry
/// attack. Certificate re-verification stays as defence in depth.
fn read_device_certs(
    doc: &AutoCommit,
    group_id: &[u8],
    owner_id: &DeviceId,
) -> Vec<DeviceCertificate> {
    let mut out = Vec::new();
    let Ok(Some((Value::Object(ObjType::Map), devices))) = doc.get(ROOT, D_DEVICES) else {
        return out;
    };
    for fp in doc.keys(&devices) {
        let Ok(Some((Value::Object(ObjType::Map), entry))) = doc.get(&devices, &fp) else {
            continue;
        };
        let cert_bytes = bytes_field(doc, &entry, D_CERT);
        let Ok(cert) = DeviceCertificate::decode(&cert_bytes) else {
            continue;
        };
        // The current owner must have signed this exact (group, companion, certificate) entry.
        let owner_pk = bytes_field(doc, &entry, D_OWNER_PK);
        let owner_sig = bytes_field(doc, &entry, D_OWNER_SIG);
        let Ok(sig): Result<[u8; 64], _> = owner_sig.as_slice().try_into() else {
            continue;
        };
        if DeviceId::from_public_key_bytes(&owner_pk) != *owner_id
            || !verify_with_public_bytes(
                &owner_pk,
                &device_entry_payload(group_id, &fp, &cert_bytes),
                &sig,
            )
        {
            continue;
        }
        if cert.group_id != group_id
            || !cert.verify(&cert.origin_id)
            || fingerprint(&cert.new_device_id) != fp
            || fingerprint(&cert.origin_id) != str_field(doc, &entry, D_ORIGIN)
            || cert.device_name != str_field(doc, &entry, D_NAME)
        {
            continue;
        }
        out.push(cert);
        if out.len() >= MAX_DEVICES {
            break;
        }
    }
    out
}

/// Every genuine device revocation in the registry (**M5**; empty until the revocation verb
/// lands, since nothing writes [`D_REVOCATIONS`] yet). Same re-derivation rule as the
/// certificates: only a revocation actually signed by the origin it names counts, so a member
/// cannot evict someone else's device by editing the CRDT.
fn read_device_revocations(doc: &AutoCommit) -> Vec<DeviceRevocation> {
    let mut out = Vec::new();
    let Ok(Some((Value::Object(ObjType::Map), revs))) = doc.get(ROOT, D_REVOCATIONS) else {
        return out;
    };
    for fp in doc.keys(&revs) {
        let Ok(Some((Value::Object(ObjType::Map), entry))) = doc.get(&revs, &fp) else {
            continue;
        };
        let Ok(rev) = DeviceRevocation::decode(&bytes_field(doc, &entry, D_CERT)) else {
            continue;
        };
        if !rev.verify(&rev.origin_id) || fingerprint(&rev.revoked_device_id) != fp {
            continue;
        }
        out.push(rev);
        if out.len() >= MAX_DEVICES {
            break;
        }
    }
    out
}

// --- fileshare: a per-server file index --------------------------------------
//
// One shared CRDT document per server (`DocType::FileIndex`, id `FILE_INDEX_DOC`): an
// append-only list of file entries `{ name, size, mime, cid, author }`. The bytes live in
// the blob store and are fetched on demand over the mesh (8l); only the small metadata
// gossips in the (encrypted) index. NOTE: blobs are stored plaintext at rest and served
// members-only; per-file encryption-at-rest (`catcoms-storage::seal_file`) is a hardening
// follow-up; the index metadata is already confidential (it is an encrypted CRDT doc).

/// The reserved document id for the per-server file index.
const FILE_INDEX_DOC: u128 = 0;
/// The reserved document id for the per-server status feed (`DocType::Status`).
const STATUS_DOC: u128 = 0;
/// The reserved document id for the per-server wiki (`DocType::Wiki`); one doc that is a
/// map of page name → page body.
const WIKI_DOC: u128 = 0;

/// The reserved root key holding the wiki's **per-page metadata** map (`page name -> format`).
///
/// NUL-prefixed on purpose: a NUL is untypeable as a page name, so it can never collide with a
/// real page, and; because it holds an automerge **`Map`**, not a `Text`; [`read_wiki_map`]
/// (which only materializes `Text` values) is blind to it. That is the backward-compatibility
/// mechanism: an older peer that has never heard of formats merges this key through the CRDT
/// without ever showing it as a page.
const WIKI_META_KEY: &str = "\u{0}meta";

/// Maximum length of a wiki page name, in characters; the cap the frontend's `[[link]]` grammar
/// enforces, mirrored here so an over-long name can never be *written* either.
pub const MAX_WIKI_NAME_CHARS: usize = 120;

/// The render formats a page may declare. A page **absent** from the metadata map has no
/// declared format and renders as markdown; the default is "missing", never a written value,
/// so a doc written before formats existed reads correctly.
const WIKI_FORMATS: [&str; 2] = ["md", "wiki"];

/// Write a wiki page's body. Each page body is an automerge **`Text`** object, so concurrent
/// edits to the *same* page **merge character-by-character** (a real collaborative CRDT
/// document), not last-writer-wins. `update_text` diffs the stored text against `body` and
/// splices the minimal change, so two members' edits both survive on convergence.
fn write_wiki_page(doc: &mut AutoCommit, name: &str, body: &str) -> Result<(), AutomergeError> {
    let text = match doc.get(ROOT, name)? {
        Some((Value::Object(ObjType::Text), id)) => id,
        _ => doc.put_object(ROOT, name, ObjType::Text)?,
    };
    doc.update_text(&text, body)?;
    Ok(())
}

/// Materialize the wiki document into a `page name -> body` map (each body a `Text` object).
///
/// Reserved (NUL-prefixed) root keys are skipped explicitly; defence in depth beside the
/// `Text`-only type filter, so a reserved key can never surface as a page even if a future
/// schema stores something text-shaped under one.
fn read_wiki_map(doc: &AutoCommit) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for key in doc.keys(ROOT) {
        if key.starts_with('\u{0}') {
            continue;
        }
        if let Ok(Some((Value::Object(ObjType::Text), id))) = doc.get(ROOT, &key) {
            if let Ok(body) = doc.text(&id) {
                out.insert(key, body);
            }
        }
    }
    out
}

/// Trim and validate a wiki page name: non-empty, not reserved (no leading NUL), and within
/// [`MAX_WIKI_NAME_CHARS`]; the same cap the frontend's `[[link]]` grammar enforces, so a name
/// that can be written is always a name that can be linked.
fn valid_wiki_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Invalid("empty wiki page name".into()));
    }
    if name.starts_with('\u{0}') {
        return Err(AppError::Invalid("reserved wiki page name".into()));
    }
    if name.chars().count() > MAX_WIKI_NAME_CHARS {
        return Err(AppError::Invalid(format!(
            "wiki page name longer than {MAX_WIKI_NAME_CHARS} characters"
        )));
    }
    Ok(name.to_string())
}

/// Every metadata map currently living at the reserved key, in automerge's conflict order;
/// the **last** is the winner `doc.get` would return.
///
/// There is normally exactly one. But the map is created lazily by whoever first sets a format,
/// so two members who do that concurrently each `put_object` a *fresh* `Map` at the same root
/// key; on merge automerge keeps both objects and picks one winner. Reads therefore union all
/// of them (winner last, so it takes precedence) and deletes hit all of them; otherwise a
/// format set on the losing side would silently vanish, or a deleted page's entry resurface.
fn wiki_meta_objs(doc: &AutoCommit) -> Vec<ObjId> {
    reserved_map_objs(doc, WIKI_META_KEY)
}

/// Get (or create) the wiki's metadata map; the conflict winner if several exist.
fn wiki_meta_obj(doc: &mut AutoCommit) -> Result<ObjId, AutomergeError> {
    reserved_map_obj(doc, WIKI_META_KEY)
}

/// Record a page's render `format` (`"md"` or `"wiki"`). A plain last-writer-wins scalar put:
/// a format is a toggle, so a concurrent flip resolving to one of the two values is correct;
/// unlike the body, which merges character-by-character.
fn set_wiki_format(doc: &mut AutoCommit, name: &str, format: &str) -> Result<(), AutomergeError> {
    let meta = wiki_meta_obj(doc)?;
    doc.put(&meta, name, format)?;
    Ok(())
}

/// Materialize the wiki's `page name -> format` metadata (empty if the doc has none). A page
/// absent from this map has no declared format and renders as markdown.
fn read_wiki_meta(doc: &AutoCommit) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for meta in wiki_meta_objs(doc) {
        for key in doc.keys(&meta) {
            let format = str_field(doc, &meta, &key);
            if WIKI_FORMATS.contains(&format.as_str()) {
                out.insert(key, format);
            }
        }
    }
    out
}

/// Delete a wiki page; its body and its metadata entry. The body delete is guarded: a page
/// can exist only as an accepted pending edit (review mode), with no stored body yet.
fn delete_wiki_page_op(doc: &mut AutoCommit, name: &str) -> Result<(), AutomergeError> {
    if doc.get(ROOT, name)?.is_some() {
        doc.delete(ROOT, name)?;
    }
    for meta in wiki_meta_objs(doc) {
        if doc.get(&meta, name)?.is_some() {
            doc.delete(&meta, name)?;
        }
    }
    Ok(())
}

/// Rename a wiki page: copy its body to `to`, carry its format, delete `from`.
///
/// automerge has no "move", so this is a **copy + delete**, and the new `Text` object is a fresh
/// CRDT identity. Consequence, accepted deliberately: an edit made concurrently on the *old* key
/// is lost by the rename (it merges into a page that no longer exists), where an edit made
/// concurrently on the *new* key merges character-by-character as usual. Renames are rare and
/// deliberate; the alternative (keeping a tombstone alias) buys little for the complexity.
fn rename_wiki_page_op(doc: &mut AutoCommit, from: &str, to: &str) -> Result<(), AutomergeError> {
    let body = match doc.get(ROOT, from)? {
        Some((Value::Object(ObjType::Text), id)) => doc.text(&id)?,
        _ => String::new(),
    };
    write_wiki_page(doc, to, &body)?;
    let format = read_wiki_meta(doc).get(from).cloned();
    if let Some(format) = format {
        set_wiki_format(doc, to, &format)?;
    }
    delete_wiki_page_op(doc, from)
}

// --- wiki history + reviewed (pending) edits (11x) ---------------------------
//
// Three more reserved NUL-prefixed root keys beside `\u{0}meta`, invisible to the page reader
// (`read_wiki_map` skips NUL keys and non-`Text` values) and merged straight through by older
// peers, so none of this disturbs a client that predates the feature:
//
// * `\u{0}hist`    - `page name -> { revision id -> { author, ts, body, kind, actor, note } }`.
//   Every materialized change appends a revision holding the **full body snapshot** (pages are
//   small text; a snapshot makes diff/rollback trivial and survives merges losslessly).
// * `\u{0}pending` - `pending id -> { page, author, ts, expires, body }`: member edits awaiting
//   review while the server's review mode is on.
// * `\u{0}cfg`     - wiki-wide settings; today `review_days` (0 = off), an owner/admin LWW put.
//
// **Auto-acceptance is read-time, not write-time.** A pending edit whose deadline has passed is
// treated as the page's live body by every reader deterministically (`doc + now -> state`);
// nobody races to "apply" it, which matters in a CRDT where two peers splicing the same text
// concurrently would duplicate it. The expired entry is folded into `\u{0}hist` lazily, by the
// next *direct* write to that page, under a **deterministic revision id (= the pending id)** so
// two peers folding concurrently converge on one history entry.
//
// Like the meta map, each container is created lazily, so two peers can concurrently `put_object`
// it and automerge keeps both with one winner: reads union **all** conflicting containers
// (winner last) and deletes hit all of them.

/// The reserved root key holding the wiki's revision history (`page -> rev id -> revision`).
const WIKI_HIST_KEY: &str = "\u{0}hist";
/// The reserved root key holding pending (review-mode) edits (`pending id -> pending edit`).
const WIKI_PENDING_KEY: &str = "\u{0}pending";
/// The reserved root key holding wiki-wide settings (`review_days`).
const WIKI_CFG_KEY: &str = "\u{0}cfg";
/// The `\u{0}cfg` field: days a member edit waits for review before auto-accepting; 0 = off.
const WCFG_REVIEW_DAYS: &str = "review_days";
/// The most a review window may be: an unreviewed edit auto-accepts after at most this long.
pub const MAX_WIKI_REVIEW_DAYS: u32 = 30;
/// One day in milliseconds (review windows are whole days).
const DAY_MS: u64 = 86_400_000;

const WH_AUTHOR: &str = "author";
const WH_TS: &str = "ts";
const WH_BODY: &str = "body";
const WH_KIND: &str = "kind";
const WH_ACTOR: &str = "actor";
const WH_NOTE: &str = "note";

const WP_PAGE: &str = "page";
const WP_AUTHOR: &str = "author";
const WP_TS: &str = "ts";
const WP_EXPIRES: &str = "expires";
const WP_BODY: &str = "body";

/// One entry in a page's revision history, newest-last as [`read_wiki_history`] returns them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WikiRevision {
    /// Stable revision id. For a revision born from a pending edit (approved / auto-accepted /
    /// rejected) this **is** the pending id, so concurrent folding converges on one entry.
    pub id: String,
    /// The **proposer's** device fingerprint (who wrote the words), resolved to a display name
    /// at render time like a message author.
    pub author: String,
    /// When the revision took effect, epoch-millis (for an auto-accepted edit, its deadline).
    pub ts: u64,
    /// The full page body as of this revision.
    pub body: String,
    /// What happened: `"edit"`, `"approve"`, `"auto"`, `"reject"`, `"rollback"`, `"delete"`,
    /// or `"rename"`. A `"reject"` revision records the *proposed* body that was declined; it
    /// was never live.
    pub kind: String,
    /// The reviewer's fingerprint for `"approve"`/`"reject"`; empty otherwise.
    pub actor: String,
    /// Context: the old name for `"rename"`, the restored revision id for `"rollback"`.
    pub note: String,
}

/// A member edit sitting in the review queue.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WikiPendingEdit {
    /// Stable id (random hex, minted like a message id).
    pub id: String,
    /// The page this edit targets (which may not exist yet: page creation queues too).
    pub page: String,
    /// The proposer's device fingerprint.
    pub author: String,
    /// When it was submitted, epoch-millis.
    pub ts: u64,
    /// When it auto-accepts if nobody reviews it, epoch-millis.
    pub expires_ts: u64,
    /// The proposed full page body.
    pub body: String,
}

/// Every map currently living at reserved root key `key`, in automerge's conflict order (the
/// **last** is the winner `doc.get` would return). Same union-read rationale as
/// [`wiki_meta_objs`]: lazily-created containers can conflict, and a read must see all of them.
fn reserved_map_objs(doc: &AutoCommit, key: &str) -> Vec<ObjId> {
    doc.get_all(ROOT, key)
        .into_iter()
        .flatten()
        .filter_map(|(v, id)| matches!(v, Value::Object(ObjType::Map)).then_some(id))
        .collect()
}

/// Get (or create) the winner map at reserved root key `key`.
fn reserved_map_obj(doc: &mut AutoCommit, key: &str) -> Result<ObjId, AutomergeError> {
    match doc.get(ROOT, key)? {
        Some((Value::Object(ObjType::Map), id)) => Ok(id),
        _ => doc.put_object(ROOT, key, ObjType::Map),
    }
}

/// Append one revision to `page`'s history. Writing the same revision id twice (concurrent
/// folding of the same expired pending) converges: both writers store identical fields, and
/// the id keys one entry.
fn append_wiki_rev(
    doc: &mut AutoCommit,
    page: &str,
    rev: &WikiRevision,
) -> Result<(), AutomergeError> {
    let hist = reserved_map_obj(doc, WIKI_HIST_KEY)?;
    let pmap = match doc.get(&hist, page)? {
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(&hist, page, ObjType::Map)?,
    };
    let entry = match doc.get(&pmap, rev.id.as_str())? {
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(&pmap, rev.id.as_str(), ObjType::Map)?,
    };
    doc.put(&entry, WH_AUTHOR, rev.author.as_str())?;
    doc.put(&entry, WH_TS, rev.ts as i64)?;
    doc.put(&entry, WH_BODY, rev.body.as_str())?;
    doc.put(&entry, WH_KIND, rev.kind.as_str())?;
    doc.put(&entry, WH_ACTOR, rev.actor.as_str())?;
    doc.put(&entry, WH_NOTE, rev.note.as_str())?;
    Ok(())
}

/// Materialize `page`'s stored revision history, oldest first (ties broken by id, so every
/// member reads the same order). Unions all conflicting containers at both levels; a revision
/// id seen twice keeps the winner's copy.
fn read_wiki_history(doc: &AutoCommit, page: &str) -> Vec<WikiRevision> {
    let mut by_id: HashMap<String, WikiRevision> = HashMap::new();
    for hist in reserved_map_objs(doc, WIKI_HIST_KEY) {
        let pmaps: Vec<ObjId> = doc
            .get_all(&hist, page)
            .into_iter()
            .flatten()
            .filter_map(|(v, id)| matches!(v, Value::Object(ObjType::Map)).then_some(id))
            .collect();
        for pmap in pmaps {
            for id in doc.keys(&pmap) {
                if let Ok(Some((Value::Object(ObjType::Map), entry))) = doc.get(&pmap, &id) {
                    by_id.insert(
                        id.clone(),
                        WikiRevision {
                            id,
                            author: str_field(doc, &entry, WH_AUTHOR),
                            ts: int_field(doc, &entry, WH_TS),
                            body: str_field(doc, &entry, WH_BODY),
                            kind: str_field(doc, &entry, WH_KIND),
                            actor: str_field(doc, &entry, WH_ACTOR),
                            note: str_field(doc, &entry, WH_NOTE),
                        },
                    );
                }
            }
        }
    }
    let mut out: Vec<WikiRevision> = by_id.into_values().collect();
    out.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.id.cmp(&b.id)));
    out
}

/// Store one pending edit in the review queue.
fn write_wiki_pending(doc: &mut AutoCommit, p: &WikiPendingEdit) -> Result<(), AutomergeError> {
    let pend = reserved_map_obj(doc, WIKI_PENDING_KEY)?;
    let entry = match doc.get(&pend, p.id.as_str())? {
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(&pend, p.id.as_str(), ObjType::Map)?,
    };
    doc.put(&entry, WP_PAGE, p.page.as_str())?;
    doc.put(&entry, WP_AUTHOR, p.author.as_str())?;
    doc.put(&entry, WP_TS, p.ts as i64)?;
    doc.put(&entry, WP_EXPIRES, p.expires_ts as i64)?;
    doc.put(&entry, WP_BODY, p.body.as_str())?;
    Ok(())
}

/// Every pending edit in the doc (expired ones included; callers split on `expires_ts`),
/// oldest first, ties broken by id.
fn read_wiki_pending_all(doc: &AutoCommit) -> Vec<WikiPendingEdit> {
    let mut by_id: HashMap<String, WikiPendingEdit> = HashMap::new();
    for pend in reserved_map_objs(doc, WIKI_PENDING_KEY) {
        for id in doc.keys(&pend) {
            if let Ok(Some((Value::Object(ObjType::Map), entry))) = doc.get(&pend, &id) {
                let page = str_field(doc, &entry, WP_PAGE);
                if page.is_empty() {
                    continue; // a cleared/malformed entry is not a pending edit
                }
                by_id.insert(
                    id.clone(),
                    WikiPendingEdit {
                        id,
                        page,
                        author: str_field(doc, &entry, WP_AUTHOR),
                        ts: int_field(doc, &entry, WP_TS),
                        expires_ts: int_field(doc, &entry, WP_EXPIRES),
                        body: str_field(doc, &entry, WP_BODY),
                    },
                );
            }
        }
    }
    let mut out: Vec<WikiPendingEdit> = by_id.into_values().collect();
    out.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.id.cmp(&b.id)));
    out
}

/// Remove a pending edit from **all** conflicting queue containers (like the meta delete).
fn delete_wiki_pending(doc: &mut AutoCommit, id: &str) -> Result<(), AutomergeError> {
    for pend in reserved_map_objs(doc, WIKI_PENDING_KEY) {
        if doc.get(&pend, id)?.is_some() {
            doc.delete(&pend, id)?;
        }
    }
    Ok(())
}

/// Re-target every pending edit aimed at page `from` to page `to` (a rename keeps the queue
/// meaningful; an orphaned proposal would otherwise resurrect the old name at its deadline).
fn repoint_wiki_pending(doc: &mut AutoCommit, from: &str, to: &str) -> Result<(), AutomergeError> {
    for pend in reserved_map_objs(doc, WIKI_PENDING_KEY) {
        let ids: Vec<String> = doc.keys(&pend).collect();
        for id in ids {
            if let Ok(Some((Value::Object(ObjType::Map), entry))) = doc.get(&pend, &id) {
                if str_field(doc, &entry, WP_PAGE) == from {
                    doc.put(&entry, WP_PAGE, to)?;
                }
            }
        }
    }
    Ok(())
}

/// The configured review window in days (0 = review off). Unions conflicting config maps,
/// winner last; junk or out-of-range values read as "off" rather than surprising behavior.
fn read_wiki_review_days(doc: &AutoCommit) -> u32 {
    let mut days = 0u32;
    for cfg in reserved_map_objs(doc, WIKI_CFG_KEY) {
        let v = int_field(doc, &cfg, WCFG_REVIEW_DAYS);
        if v <= MAX_WIKI_REVIEW_DAYS as u64 {
            days = v as u32;
        }
    }
    days
}

/// Set the review window (a plain LWW put; a toggle, like a page format).
fn write_wiki_review_days(doc: &mut AutoCommit, days: u32) -> Result<(), AutomergeError> {
    let cfg = reserved_map_obj(doc, WIKI_CFG_KEY)?;
    doc.put(&cfg, WCFG_REVIEW_DAYS, days as i64)?;
    Ok(())
}

/// The pending edits that have passed their deadline as of `now`: auto-accepted, in
/// acceptance order (oldest deadline first, ties by id). The **last** one per page is that
/// page's effective body.
fn expired_wiki_pending(doc: &AutoCommit, now: u64) -> Vec<WikiPendingEdit> {
    let mut out: Vec<WikiPendingEdit> = read_wiki_pending_all(doc)
        .into_iter()
        .filter(|p| p.expires_ts <= now)
        .collect();
    out.sort_by(|a, b| {
        a.expires_ts
            .cmp(&b.expires_ts)
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

/// Overlay auto-accepted pending edits onto the stored page map: the read-time half of
/// review mode. `map` is mutated into the **effective** wiki every reader (pages, bodies,
/// backlinks, pinning) sees.
fn overlay_accepted_pending(doc: &AutoCommit, now: u64, map: &mut HashMap<String, String>) {
    for p in expired_wiki_pending(doc, now) {
        map.insert(p.page, p.body);
    }
}

/// Fold `page`'s expired pending edits into its history (kind `"auto"`, ts = the deadline,
/// revision id = the pending id, so concurrent folds converge) and drop them from the queue.
/// Called at the head of every direct write to the page; the caller then writes the new body,
/// which supersedes the accepted content the history entry preserves.
fn fold_expired_wiki_pending(
    doc: &mut AutoCommit,
    page: &str,
    now: u64,
) -> Result<(), AutomergeError> {
    for p in expired_wiki_pending(doc, now) {
        if p.page != page {
            continue;
        }
        append_wiki_rev(
            doc,
            page,
            &WikiRevision {
                id: p.id.clone(),
                author: p.author,
                ts: p.expires_ts,
                body: p.body,
                kind: "auto".into(),
                actor: String::new(),
                note: String::new(),
            },
        )?;
        delete_wiki_pending(doc, &p.id)?;
    }
    Ok(())
}

// --- server events (the calendar) -------------------------------------------
//
// One shared CRDT document per server (`DocType::Calendar`, id `CALENDAR_DOC`): a versioned map
// (like the livery/badge docs) of **event id → `{ title, body, start, end, author, created }`**.
// Each event lives under its own random id key, so two members creating events concurrently
// write *distinct* keys that both survive a merge; there is no container two writers could
// create twice and lose one of.
//
// **Any member may create an event**: an event is server *content*, like a channel, a status post
// or a channel topic; not presentation like the livery; so this is deliberately not owner/admin
// gated. Deletion is the author's, or an owner/admin's (moderation), exactly like a chat message;
// the gate is honest-client-enforced (the op log is inner-signed, so authorship of a forged write
// is attributable either way; the documented R6 residual).

/// The reserved document id for the per-server calendar document.
const CALENDAR_DOC: u128 = 0;
/// Schema version written into every calendar doc, so a later shape can be told apart.
const CALENDAR_VERSION: i64 = 1;
const C_V: &str = "v";
/// The map of `event id -> { … }` inside the calendar document. Kept under its own key (like the
/// badge map) so the schema version can live beside it without ever colliding with an event id.
const C_EVENTS: &str = "events";
const C_TITLE: &str = "title";
const C_BODY: &str = "body";
const C_START: &str = "start";
const C_END: &str = "end";
const C_AUTHOR: &str = "author";
const C_CREATED: &str = "created";
/// The event's optional poster image, stored as the file's lowercase hex content address (the
/// same address a `![alt](cid:HEX)` embed names), never as inline bytes: the calendar document
/// is replicated to every member, so it carries the pointer and the blob travels the file path.
const C_IMAGE: &str = "image";

/// Maximum length of a server-event title, in UTF-8 bytes. Events live in a document every
/// member replicates, so; like the channel topic and the livery values; they are size-bounded.
pub const MAX_EVENT_TITLE_BYTES: usize = 120;
/// Maximum length of a server-event body (its longer description), in UTF-8 bytes.
pub const MAX_EVENT_BODY_BYTES: usize = 1024;
/// Maximum length of a server-event image reference: one 32-byte content address in hex.
pub const MAX_EVENT_IMAGE_HEX: usize = 64;

/// One scheduled server event as the UI sees it. The `author` is the creator's **device
/// fingerprint** (the key its [`Profile`] is stored under), resolved to a display name at
/// render time exactly like a message author.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerEvent {
    /// A stable per-event id (random hex), minted like a message id; so a delete addresses
    /// exactly one event under concurrent merges.
    pub id: String,
    /// The event title (never empty in a stored entry).
    pub title: String,
    /// A longer description; empty if none.
    pub body: String,
    /// When the event starts, epoch-millis.
    pub start_ts: u64,
    /// When the event ends, epoch-millis; `0` = no end time.
    pub end_ts: u64,
    /// The creator's device fingerprint.
    pub author: String,
    /// When the event was created, epoch-millis (the creator's injected clock).
    pub created_ts: u64,
    /// The lowercase hex content address of the event's poster image, or empty for none. The
    /// blob is fetched over the file path like any other embed, so an event whose image has not
    /// arrived yet still reads fine; it just shows without a picture.
    pub image: String,
}

/// Write one event entry into the calendar document, keyed by its id.
fn write_event(doc: &mut AutoCommit, e: &ServerEvent) -> Result<(), AutomergeError> {
    doc.put(ROOT, C_V, CALENDAR_VERSION)?;
    let events = match doc.get(ROOT, C_EVENTS)? {
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(ROOT, C_EVENTS, ObjType::Map)?,
    };
    let entry = match doc.get(&events, e.id.as_str())? {
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(&events, e.id.as_str(), ObjType::Map)?,
    };
    doc.put(&entry, C_TITLE, e.title.as_str())?;
    doc.put(&entry, C_BODY, e.body.as_str())?;
    doc.put(&entry, C_START, e.start_ts as i64)?;
    doc.put(&entry, C_END, e.end_ts as i64)?;
    doc.put(&entry, C_AUTHOR, e.author.as_str())?;
    doc.put(&entry, C_CREATED, e.created_ts as i64)?;
    doc.put(&entry, C_IMAGE, e.image.as_str())?;
    Ok(())
}

/// Remove the event with `id` from the calendar document. Ids are unique keys, so this removes
/// (at most) the single matching entry. Returns whether one was there.
fn delete_event_in_doc(doc: &mut AutoCommit, id: &str) -> Result<bool, AutomergeError> {
    let Some((Value::Object(ObjType::Map), events)) = doc.get(ROOT, C_EVENTS)? else {
        return Ok(false);
    };
    if doc.get(&events, id)?.is_none() {
        return Ok(false);
    }
    doc.delete(&events, id)?;
    Ok(true)
}

/// Materialize the calendar document into events sorted by **start time ascending** (ties broken
/// by id, so every member reads the same order). A missing/foreign-shaped or title-less entry is
/// skipped, so a malformed doc degrades to "fewer events" rather than junk rows.
fn read_events(doc: &AutoCommit) -> Vec<ServerEvent> {
    let mut out = Vec::new();
    if let Ok(Some((Value::Object(ObjType::Map), events))) = doc.get(ROOT, C_EVENTS) {
        for id in doc.keys(&events) {
            if let Ok(Some((Value::Object(ObjType::Map), entry))) = doc.get(&events, &id) {
                let title = str_field(doc, &entry, C_TITLE);
                if title.is_empty() {
                    continue; // a cleared/malformed entry is not an event
                }
                out.push(ServerEvent {
                    id,
                    title,
                    body: str_field(doc, &entry, C_BODY),
                    start_ts: int_field(doc, &entry, C_START),
                    end_ts: int_field(doc, &entry, C_END),
                    author: str_field(doc, &entry, C_AUTHOR),
                    created_ts: int_field(doc, &entry, C_CREATED),
                    // A peer that predates the field, or one that wrote junk, reads as "no
                    // image" rather than as a broken address the UI would try to fetch.
                    image: hex_cid_field(doc, &entry, C_IMAGE),
                });
            }
        }
    }
    out.sort_by(|a, b| a.start_ts.cmp(&b.start_ts).then_with(|| a.id.cmp(&b.id)));
    out
}

// --- member roles (Phase 10h + item 3) -------------------------------------
//
// The **owner** is the MLS designated committer (the founder; cryptographically anchored, not
// stored); everyone else is a plain member or an **admin**.
//
// ENFORCEMENT (item 3; replay-proof revocation; see `docs/design-grant-revocation.md`):
//
// * **Owner** is the MLS designated committer (lowest leaf index); only that device can act
//   as committer, so the owner is cryptographically anchored, not a stored/forgeable field.
//   It is NOT sticky: it follows the lowest *live* leaf, so if the founder ever leaves the
//   group, ownership (and admin-granting power) passes to the next-lowest member. (Founder
//   removal is not wired into the desktop app yet, so this is latent.) A new owner starts with
//   an empty roster (prior grants lapse until re-granted); no stale-grant inheritance.
//
// * **Admin** authority is the **owner's local authoritative roster** (`ChannelSync::admin_roster`,
//   persisted in the snapshot). Because only the owner runs admission (Option C), the gate
//   (`inviter_is_authorized`) reads that local set, which a malicious member cannot write; so a
//   demoted admin re-adding/replaying its old grant into the shared CRDT can no longer
//   re-authorize itself (closes the old revocation-by-deletion residual). The owner publishes a
//   single owner-signed `roster` value into `DocType::MemberRoles` for **display only**; readers
//   verify the owner's signature, so a tampered copy is at worst cosmetic (never an admission).
//
// Residual: a tampered/stale published roster can transiently mislead *other members'* role
// badges (cosmetic, R4-class). The guarantee rests on single-committer admission; do NOT enable
// `max_committer_rank ≥ 1` (a second committer would re-introduce the replay surface).

// `ROLES_DOC`, `roster_payload`, `encode_roster`, `read_published_roster`, and `fingerprint` live
// in `catcoms-sync` (next to the admission gate that enforces them) and are re-exported here.

/// A member's effective role in a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The server owner (the MLS designated committer / founder).
    Owner,
    /// An admin (granted by the owner); may mint invites.
    Admin,
    /// A regular member.
    Member,
}

impl Role {
    /// The lowercase wire/UI string for this role.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }
    /// Whether this role may mint invites + manage admins.
    pub fn can_invite(self) -> bool {
        matches!(self, Role::Owner | Role::Admin)
    }
}

const FILES: &str = "files";
const F_NAME: &str = "name";
const F_AUTHOR: &str = "author";
// 10c: a virtual folder path for organisation (e.g. "", "docs", "embed/<fp>", "wiki/<page>",
// "emoji"). The blob itself is still content-addressed; the path is mutable index metadata.
const F_PATH: &str = "path";
// 9h: the encoded FileRef (ciphertext CID + wrapped per-file key + plaintext CID + size +
// mime). The file's bytes are stored/shared as ciphertext keyed by the ciphertext CID; only
// members with the group file-wrap key can open it. Size/mime/cid are read back from here.
const F_REF: &str = "ref";
// Optional v2 origin attestation. Older listings remain readable but are not eligible for
// specific-uploader automatic media trust, because their author label was only CRDT metadata.
const F_SIGNER_KEY: &str = "signer_key";
const F_SIGNATURE: &str = "signature";
const FILE_ENTRY_ATTEST_DOMAIN: &[u8] = b"catcoms/file-entry-attestation/v1";
// Circulation expiry for THIS listing (see [`FileExpiry`]): absent = never recorded (a listing
// written before expiry existed), an explicit `null` = keep forever, an integer = the absolute
// ms-epoch deadline. Additive: a reader that predates the field ignores it, and a listing
// written by such a peer simply decodes as "not recorded".
const F_EXPIRES: &str = "exp";

/// How long a shared file stays in **circulation**; the default lifetime stamped on every new
/// listing by [`Server::add_file`].
///
/// One month, matching `catcoms_storage::ONE_MONTH_MS` (the retention engine's global default);
/// re-declared here rather than imported so the app-layer stamp and the storage-layer policy can
/// drift apart deliberately rather than by accident.
pub const FILE_EXPIRY_DEFAULT_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// A listing's circulation deadline.
///
/// **What "expired" means (product rule):** the file stops being auto-circulated / auto-shared.
/// It is **never deleted** from anyone's disk, and it stays fetchable by cid for as long as any
/// peer still holds its chunks. Expiry is per **listing**, not per content: the same bytes listed
/// under two names/folders (content dedup in [`Server::add_file`]) carry two independent
/// deadlines.
///
/// **NOTHING ENFORCES THIS YET.** As of this change the deadline is *recorded and displayed*
/// metadata only; no eviction, no drop-from-circulation, no GC consults it. `catcoms-storage`'s
/// [`RetentionIndex`](catcoms_storage::RetentionIndex) is a complete 3-scope expiry + GC engine,
/// but it is **not wired into this layer at all**: the shared-file index records expiry, the blob
/// store evicts nothing, and an "expired" file keeps circulating exactly as before. The point of
/// stamping it now is that the eventual enforcement pass has honest inputs; including
/// [`Server::wiki_pinned_cids`], which it MUST consult (wiki-embedded files never decay).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileExpiry {
    /// No deadline was ever recorded; a listing written before this field existed, or by a peer
    /// that predates it. Distinct from [`FileExpiry::Never`]: we do not know what was intended,
    /// so the UI says "not recorded (older share)" rather than promising anything.
    Unrecorded,
    /// Explicitly kept forever ("keep forever"): never drops out of circulation.
    Never,
    /// Drops out of circulation at this absolute ms-epoch instant.
    At(u64),
}

impl FileExpiry {
    /// The deadline in ms epoch, if one is recorded. Both [`FileExpiry::Unrecorded`] and
    /// [`FileExpiry::Never`] answer `None`; use the variant itself to tell them apart.
    pub fn deadline_ms(self) -> Option<u64> {
        match self {
            FileExpiry::At(ms) => Some(ms),
            _ => None,
        }
    }
    /// Whether a deadline (or an explicit keep-forever) was ever recorded for this listing.
    pub fn is_recorded(self) -> bool {
        !matches!(self, FileExpiry::Unrecorded)
    }
}

/// Maximum file size accepted by [`Server::add_file`]. Chunked transfer splits a file into
/// [`CHUNK_BYTES`] pieces, so this is a whole-**file** cap (256 MiB), no longer the per-blob
/// transport limit. (GB-scale + background/streaming download is a follow-up.)
pub const MAX_FILE_BYTES: usize = 256 * 1024 * 1024;

/// Maximum number of file-index rows this implementation publishes or materializes. A current
/// malicious member can still append CRDT rows, but rows beyond this deterministic boundary do
/// not trigger unbounded manifest parsing or Ed25519 verification on every file-list refresh.
pub const MAX_FILE_ENTRIES: usize = 256;
/// Maximum number of distinct wrapped-key/reference claims storage inspection will authenticate
/// against one ciphertext CID. Honest deduplication reuses a byte-identical reference, while a
/// larger set can only be contradictory or adversarial. Failing that CID closed prevents one held
/// 8 MiB blob from being decrypted thousands of times inside the single server actor.
const MAX_FILE_REF_VARIANTS_PER_CID: usize = 4;
const MAX_FILE_NAME_BYTES: usize = 255;
const MAX_FILE_AUTHOR_BYTES: usize = 128;
const MAX_FILE_PATH_BYTES: usize = 2048;
const MAX_FILE_MIME_BYTES: usize = 255;
const MAX_FILE_REF_BYTES: usize = 64 * 1024;

/// Plaintext chunk size for large-file transfer. Chosen well under the blob-fetch response cap
/// (`MAX_BLOB_RESPONSE` = 16 MiB) so each *sealed* chunk (≈ chunk + ~44 B) fits one response.
///
/// It is **the same number** as the size-quantization ladder's ceiling
/// ([`catcoms_storage::CHUNK_PAD_CEILING`]), and the equality is load-bearing rather than
/// coincidental: it is what makes a full chunk a fixed point of the padding (cost: the 4-byte
/// length footer, nothing else) while every short tail chunk pads up to it, so a large file's
/// exact size stops leaking through its tail. Pinned by
/// `the_chunk_size_is_the_padding_ladders_ceiling`; if the two ever drift, a full chunk starts
/// paying a whole bucket step and the product's bulk traffic doubles.
pub const CHUNK_BYTES: usize = catcoms_storage::CHUNK_PAD_CEILING;

// A manifest's chunk count is a hostile-input parse guard set to the product's true maximum, so
// raising the file-size limit without raising it would make the largest legal file undecodable.
const _: () = assert!(MAX_FILE_BYTES.div_ceil(CHUNK_BYTES) <= catcoms_storage::MAX_CHUNKS as usize);

/// One window of a shared file's plaintext, as served to the media protocol.
#[derive(Debug, Clone)]
pub struct FileRange {
    /// The requested plaintext bytes, clamped to the end of the file.
    pub bytes: Vec<u8>,
    /// The file's full plaintext length, for `Content-Range`.
    pub total_size: u64,
    /// The declared MIME type (best-effort; may be empty).
    pub mime: String,
    /// The peer that served a chunk this range needed, or `None` when every chunk it touched was
    /// already held locally. This is what lets the UI say "loading" rather than "downloading".
    pub provider: Option<String>,
}

fn file_manifest_version(file_ref: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mewtual-file-media-manifest/v1");
    hasher.update(&(file_ref.len() as u64).to_be_bytes());
    hasher.update(file_ref);
    *hasher.finalize().as_bytes()
}

/// Stable identity of one exact encrypted-chunk reference, including its wrapped key, declared
/// size and MIME. A ciphertext CID alone identifies only the sealed bytes: a hostile listing can
/// repeat that CID with a different key/ref, so inventory authentication must bind its verdict to
/// all of the fields that [`ChannelSync::open_file`] consumes.
fn file_chunk_ref_version(file_ref: &FileRef) -> [u8; 32] {
    let encoded = file_ref.encode();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mewtual-file-chunk-ref/v1");
    hasher.update(&(encoded.len() as u64).to_be_bytes());
    hasher.update(&encoded);
    *hasher.finalize().as_bytes()
}

/// Bounded exact references for one ciphertext address.
///
/// Once the cap is crossed all variants fail closed, including variants authenticated earlier in
/// the scan. Which hostile row happened to arrive first must never decide which manifest the
/// storage pane calls verified.
#[derive(Debug)]
enum BoundedFileRefs {
    Exact(HashMap<[u8; 32], FileRef>),
    Ambiguous,
}

impl Default for BoundedFileRefs {
    fn default() -> Self {
        Self::Exact(HashMap::new())
    }
}

impl BoundedFileRefs {
    fn insert(&mut self, version: [u8; 32], file_ref: FileRef) {
        let Self::Exact(refs) = self else {
            return;
        };
        if refs.contains_key(&version) {
            return;
        }
        if refs.len() >= MAX_FILE_REF_VARIANTS_PER_CID {
            *self = Self::Ambiguous;
            return;
        }
        refs.insert(version, file_ref);
    }
}

type StorageRefIndex = (
    HashMap<Cid, BoundedFileRefs>,
    Vec<([u8; 32], Vec<[u8; 32]>)>,
    usize,
);

/// Parse the bounded materialized index once into exact per-CID claims and per-manifest
/// dependencies. The returned maps are themselves bounded by `MAX_FILE_ENTRIES * MAX_CHUNKS`;
/// contradictory claims for one CID collapse to `Ambiguous` instead of retaining attacker-sized
/// variant sets.
fn storage_ref_index(files: &[FileEntry]) -> StorageRefIndex {
    let mut refs = HashMap::<Cid, BoundedFileRefs>::new();
    let mut file_refs = Vec::<([u8; 32], Vec<[u8; 32]>)>::new();
    let mut invalid_manifests = 0;
    for entry in files {
        match FileManifest::decode_or_legacy(&entry.file_ref) {
            Ok(manifest) if manifest.total_size <= MAX_FILE_BYTES as u64 => {
                let chunk_versions = manifest
                    .chunks
                    .iter()
                    .map(file_chunk_ref_version)
                    .collect::<Vec<_>>();
                file_refs.push((file_manifest_version(&entry.file_ref), chunk_versions));
                for chunk in manifest.chunks {
                    let version = file_chunk_ref_version(&chunk);
                    refs.entry(chunk.ciphertext_cid)
                        .or_default()
                        .insert(version, chunk);
                }
            }
            _ => invalid_manifests += 1,
        }
    }
    (refs, file_refs, invalid_manifests)
}

/// Index metadata that authorizes one inline-media representation.
///
/// `manifest_version` binds every later range read to the exact chunk manifest that supplied the
/// size and MIME. The plaintext CID is member-authored until a whole-file download verifies it,
/// so it cannot safely serve as that version by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMediaHead {
    pub total_size: u64,
    pub mime: String,
    pub manifest_version: [u8; 32],
}

/// One shared file as the UI sees it. `cid` is the **whole-file plaintext** content address (raw
/// bytes); the file's stable identity / download+embed handle (a chunked file has no single
/// ciphertext blob); `author` is the uploader's short display fingerprint, while
/// `author_identity` is the full device identity used for authorization. The file's bytes are
/// end-to-end encrypted under the group file-wrap key (Phase 9h), chunk by chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// The file's display name.
    pub name: String,
    /// Plaintext size in bytes.
    pub size: u64,
    /// MIME type (best-effort; may be empty).
    pub mime: String,
    /// Content address of the **ciphertext** blob (raw 32 bytes); the download handle.
    pub cid: Vec<u8>,
    /// The uploader's device fingerprint.
    pub author: String,
    /// Full content-addressed device identity of the valid attestation signer, or empty for a
    /// legacy/invalid listing. Never authorize with the 32-bit display fingerprint above.
    pub author_identity: String,
    /// Whether the signer is bound to this exact group/name/path/file reference and its short
    /// display fingerprint matches `author`.
    pub author_verified: bool,
    /// A virtual folder path for organisation (`""` = root). Embeds live under
    /// `embed/<fp>`, wiki media under `wiki/<page>`, custom emoji under `emoji` (10c–10f).
    pub path: String,
    /// The encoded [`FileRef`] (wrapped per-file key + addresses) needed to decrypt. Carried
    /// in the encrypted index; not forwarded to the UI.
    pub file_ref: Vec<u8>,
    /// When THIS listing drops out of circulation; see [`FileExpiry`] for the three states and
    /// for the blunt truth that nothing enforces it yet.
    pub expires: FileExpiry,
}

/// A listed file plus how many of its chunks this device already holds locally, for the file
/// browser's availability indicator. `held_chunks == total_chunks` ⇒ openable with no network
/// fetch; `0 < held < total` ⇒ partially downloaded; `held == 0` ⇒ not yet downloaded. The counts
/// are a pure local blob-store check (zero network cost).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileListing {
    /// The file index entry (metadata).
    pub entry: FileEntry,
    /// Chunks of this file already held in the local blob store.
    pub held_chunks: u32,
    /// Total chunks the file is split into.
    pub total_chunks: u32,
    /// Digest of the exact current manifest. This is kept with the listing so a storage-health
    /// verdict can be joined to the same actor snapshot without trusting the plaintext CID.
    pub manifest_version: [u8; 32],
}

/// The shared file list with per-file local-availability counts, plus whether a live peer has
/// previously served roster-verified catch-up. `has_peers` is a cheap in-memory signal; it does
/// NOT prove a given file is held by any peer, only that an authenticated fetch could be tried.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilesView {
    /// The listed files with availability counts.
    pub files: Vec<FileListing>,
    /// Whether ≥1 previously proven catch-up source has a live connection on this device.
    pub has_peers: bool,
}

/// Verified local storage facts for the file chunks referenced by one server.
///
/// A filename existing on disk is not enough: `verified_chunks` counts only sealed records that
/// authenticate, match their content address, and decrypt through the listed file reference.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageHealth {
    pub listed_files: usize,
    pub referenced_chunks: usize,
    pub verified_chunks: usize,
    pub missing_chunks: usize,
    pub unreadable_chunks: usize,
    pub invalid_manifests: usize,
    /// Exact manifest digests whose every exact chunk reference passed sealed-record,
    /// ciphertext-address and file-layer authentication in this scan. Plaintext CIDs are not used
    /// here: another member can claim the same CID while supplying a different wrapped key/ref.
    pub verified_manifest_versions: HashSet<[u8; 32]>,
    /// Bytes of verified encrypted file-chunk content (not filesystem allocation or unrelated
    /// avatar/banner blobs).
    pub verified_bytes: u64,
    /// A live peer that proved it could answer a roster-authenticated catch-up earlier this
    /// session exists. This means repair can be attempted, not that it holds every missing chunk.
    pub has_peers: bool,
}

/// One atomic actor snapshot used by the storage pane. The health verdict and listing metadata
/// must be captured from the same index revision; two separate actor commands leave a mutation
/// window in which a verified old manifest can be paired with an unverified replacement row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageSnapshot {
    pub health: StorageHealth,
    pub files: FilesView,
}

/// Result of one explicit storage repair pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageRepair {
    pub attempted_chunks: usize,
    pub recovered_chunks: usize,
    pub health: StorageHealth,
}

/// Normalize a virtual folder path: trim whitespace + surrounding slashes and drop empty,
/// `.` and `..` segments, so `""` is the root and a path can never escape it.
fn normalize_path(path: &str) -> String {
    path.split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .collect::<Vec<_>>()
        .join("/")
}

fn push_attested_part(out: &mut Vec<u8>, part: &[u8]) {
    out.extend_from_slice(&(part.len() as u64).to_be_bytes());
    out.extend_from_slice(part);
}

/// Canonical, group-bound statement proving which device created one exact file listing.
fn file_entry_attestation_payload(
    group_id: &[u8],
    name: &str,
    author: &str,
    path: &str,
    file_ref: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        FILE_ENTRY_ATTEST_DOMAIN.len()
            + group_id.len()
            + name.len()
            + author.len()
            + path.len()
            + file_ref.len()
            + 5 * 8,
    );
    out.extend_from_slice(FILE_ENTRY_ATTEST_DOMAIN);
    for part in [
        group_id,
        name.as_bytes(),
        author.as_bytes(),
        path.as_bytes(),
        file_ref,
    ] {
        push_attested_part(&mut out, part);
    }
    out
}

/// Append a file entry (name + author + folder path + encoded `FileRef` + circulation expiry)
/// to the index doc.
fn write_file_entry(
    doc: &mut AutoCommit,
    name: &str,
    author: &str,
    path: &str,
    file_ref: &[u8],
    expires: FileExpiry,
    attestation: Option<(&[u8], &[u8])>,
) -> Result<(), AutomergeError> {
    let list = match doc.get(ROOT, FILES)? {
        Some((Value::Object(ObjType::List), id)) => id,
        _ => doc.put_object(ROOT, FILES, ObjType::List)?,
    };
    let index = doc.length(&list);
    let entry = doc.insert_object(&list, index, ObjType::Map)?;
    doc.put(&entry, F_NAME, name)?;
    doc.put(&entry, F_AUTHOR, author)?;
    doc.put(&entry, F_PATH, path)?;
    doc.put(&entry, F_REF, ScalarValue::Bytes(file_ref.to_vec()))?;
    if let Some((public_key, signature)) = attestation {
        doc.put(
            &entry,
            F_SIGNER_KEY,
            ScalarValue::Bytes(public_key.to_vec()),
        )?;
        doc.put(&entry, F_SIGNATURE, ScalarValue::Bytes(signature.to_vec()))?;
    }
    put_expiry(doc, &entry, expires)?;
    Ok(())
}

/// Write a listing's circulation expiry. [`FileExpiry::Unrecorded`] leaves the key **absent**;
/// that is exactly what a legacy entry looks like, so it is the one state we never fabricate on
/// an entry that already has a deadline (`set_file_expiry` can only ask for the other two).
fn put_expiry(
    doc: &mut AutoCommit,
    entry: &ObjId,
    expires: FileExpiry,
) -> Result<(), AutomergeError> {
    match expires {
        FileExpiry::Unrecorded => {}
        // An explicit null is distinguishable from an absent key on read, which is what keeps
        // "keep forever" and "never recorded" apart across a merge.
        FileExpiry::Never => doc.put(entry, F_EXPIRES, ScalarValue::Null)?,
        FileExpiry::At(ms) => doc.put(entry, F_EXPIRES, ScalarValue::Uint(ms))?,
    }
    Ok(())
}

/// Read a listing's circulation expiry. Absent (or any unexpected scalar type) decodes as
/// [`FileExpiry::Unrecorded`], so a legacy entry; and any entry a future peer writes oddly;
/// round-trips without claiming a deadline it never had.
fn expiry_field(doc: &AutoCommit, obj: &ObjId) -> FileExpiry {
    match doc.get(obj, F_EXPIRES) {
        Ok(Some((Value::Scalar(s), _))) => match s.as_ref() {
            ScalarValue::Null => FileExpiry::Never,
            ScalarValue::Uint(u) => FileExpiry::At(*u),
            ScalarValue::Int(i) if *i >= 0 => FileExpiry::At(*i as u64),
            _ => FileExpiry::Unrecorded,
        },
        _ => FileExpiry::Unrecorded,
    }
}

/// Read a string without cloning a hostile oversized scalar out of the Automerge document.
/// Absent/wrong-type fields retain the legacy empty-string behavior; oversized fields reject the
/// row because they would make repeated materialization unbounded.
fn bounded_file_string(doc: &AutoCommit, obj: &ObjId, key: &str, max: usize) -> Option<String> {
    match doc.get(obj, key) {
        Ok(Some((Value::Scalar(value), _))) => match value.as_ref() {
            ScalarValue::Str(text) if text.len() <= max => Some(text.to_string()),
            ScalarValue::Str(_) => None,
            _ => Some(String::new()),
        },
        _ => Some(String::new()),
    }
}

/// Read bytes only after checking their borrowed length, avoiding an attacker-sized clone.
fn bounded_file_bytes(doc: &AutoCommit, obj: &ObjId, key: &str, max: usize) -> Option<Vec<u8>> {
    match doc.get(obj, key) {
        Ok(Some((Value::Scalar(value), _))) => match value.as_ref() {
            ScalarValue::Bytes(bytes) if bytes.len() <= max => Some(bytes.clone()),
            ScalarValue::Bytes(_) => None,
            _ => Some(Vec::new()),
        },
        _ => Some(Vec::new()),
    }
}

/// Materialize the file index document into the UI's file list (size/mime/cid come from the
/// decoded `FileRef`; entries with a malformed ref are skipped).
fn read_file_entries(doc: &AutoCommit, group_id: &[u8]) -> Vec<FileEntry> {
    let mut out = Vec::new();
    if let Ok(Some((Value::Object(ObjType::List), list))) = doc.get(ROOT, FILES) {
        for i in 0..doc.length(&list).min(MAX_FILE_ENTRIES) {
            if let Ok(Some((Value::Object(ObjType::Map), entry))) = doc.get(&list, i) {
                let Some(ref_bytes) = bounded_file_bytes(doc, &entry, F_REF, MAX_FILE_REF_BYTES)
                else {
                    continue;
                };
                if let Ok(manifest) = FileManifest::decode_or_legacy(&ref_bytes) {
                    if manifest.mime.len() > MAX_FILE_MIME_BYTES {
                        continue;
                    }
                    let Some(name) = bounded_file_string(doc, &entry, F_NAME, MAX_FILE_NAME_BYTES)
                    else {
                        continue;
                    };
                    let Some(author) =
                        bounded_file_string(doc, &entry, F_AUTHOR, MAX_FILE_AUTHOR_BYTES)
                    else {
                        continue;
                    };
                    let Some(path) = bounded_file_string(doc, &entry, F_PATH, MAX_FILE_PATH_BYTES)
                    else {
                        continue;
                    };
                    let public_key = bounded_file_bytes(doc, &entry, F_SIGNER_KEY, 32)
                        .filter(|bytes| bytes.len() == 32)
                        .unwrap_or_default();
                    let signature = bounded_file_bytes(doc, &entry, F_SIGNATURE, 64)
                        .filter(|bytes| bytes.len() == 64)
                        .unwrap_or_default();
                    let signer_id = DeviceId::from_public_key_bytes(&public_key);
                    let author_verified = public_key.len() == 32
                        && signature.len() == 64
                        && fingerprint(&signer_id) == author
                        && signature
                            .as_slice()
                            .try_into()
                            .is_ok_and(|signature: &[u8; 64]| {
                                verify_with_public_bytes(
                                    &public_key,
                                    &file_entry_attestation_payload(
                                        group_id, &name, &author, &path, &ref_bytes,
                                    ),
                                    signature,
                                )
                            });
                    out.push(FileEntry {
                        name,
                        author,
                        author_identity: if author_verified {
                            hex::encode(signer_id.as_bytes())
                        } else {
                            String::new()
                        },
                        author_verified,
                        size: manifest.total_size,
                        mime: manifest.mime.clone(),
                        // The file's identity is the whole-file PLAINTEXT cid (a chunked file has
                        // no single ciphertext blob); this is the download/embed handle.
                        cid: manifest.plaintext_cid.as_bytes().to_vec(),
                        path,
                        file_ref: ref_bytes,
                        expires: expiry_field(doc, &entry),
                    });
                }
            }
        }
    }
    out
}

fn raw_file_index_row_count(doc: &AutoCommit) -> usize {
    match doc.get(ROOT, FILES) {
        Ok(Some((Value::Object(ObjType::List), list))) => doc.length(&list),
        _ => 0,
    }
}

/// Remove the index entries whose whole-file plaintext CID matches `cid` (a no-op if none do),
/// restricted to folder `folder` when given. With `folder = None` *every* entry for that content
/// goes: the index is an append-only list, so a concurrent double-add can leave more than one
/// entry for the same content, and unlisting must remove them all. With `folder = Some(..)` only
/// that folder's listing goes; content dedup makes several listings of one file deliberate
/// (`add_file` re-lists shared content instead of re-storing it), so unlisting one of them must
/// leave the others alone. Iterating top-down keeps the indices we have yet to visit stable as
/// we delete. The content-addressed blobs are left in place; this only unlists.
fn delete_file_entry(
    doc: &mut AutoCommit,
    cid: &[u8],
    folder: Option<&str>,
) -> Result<(), AutomergeError> {
    let list = match doc.get(ROOT, FILES)? {
        Some((Value::Object(ObjType::List), id)) => id,
        _ => return Ok(()),
    };
    for i in (0..doc.length(&list).min(MAX_FILE_ENTRIES)).rev() {
        if let Some((Value::Object(ObjType::Map), entry)) = doc.get(&list, i)? {
            let Some(ref_bytes) = bounded_file_bytes(doc, &entry, F_REF, MAX_FILE_REF_BYTES) else {
                continue;
            };
            if let Ok(manifest) = FileManifest::decode_or_legacy(&ref_bytes) {
                let Some(path) = bounded_file_string(doc, &entry, F_PATH, MAX_FILE_PATH_BYTES)
                else {
                    continue;
                };
                let folder_matches = folder.is_none_or(|expected| path == expected);
                if manifest.plaintext_cid.as_bytes() == cid && folder_matches {
                    doc.delete(&list, i)?;
                }
            }
        }
    }
    Ok(())
}

/// Set the circulation expiry on every index entry matching `cid` **in folder `folder`** (a
/// concurrent double-add can leave more than one listing for the same content in one folder, and
/// they are the same listing as far as the user is concerned). Other folders' listings of the
/// same content keep their own deadlines; expiry is per listing, like unlisting.
fn set_file_entry_expiry(
    doc: &mut AutoCommit,
    cid: &[u8],
    folder: &str,
    expires: FileExpiry,
) -> Result<(), AutomergeError> {
    let list = match doc.get(ROOT, FILES)? {
        Some((Value::Object(ObjType::List), id)) => id,
        _ => return Ok(()),
    };
    for i in 0..doc.length(&list).min(MAX_FILE_ENTRIES) {
        if let Some((Value::Object(ObjType::Map), entry)) = doc.get(&list, i)? {
            let Some(ref_bytes) = bounded_file_bytes(doc, &entry, F_REF, MAX_FILE_REF_BYTES) else {
                continue;
            };
            if let Ok(manifest) = FileManifest::decode_or_legacy(&ref_bytes) {
                let Some(path) = bounded_file_string(doc, &entry, F_PATH, MAX_FILE_PATH_BYTES)
                else {
                    continue;
                };
                if manifest.plaintext_cid.as_bytes() == cid && path == folder {
                    put_expiry(doc, &entry, expires)?;
                }
            }
        }
    }
    Ok(())
}

/// The two marker grammars the composer emits for a shared file (desktop `refs.ts::fileMarker`):
/// `![alt](cid:HEX)`; an inline embed; and `[label](file:HEX)`; a reference chip. Both name a
/// file by its whole-file plaintext content address, so both count as *using* the file.
const FILE_MARKER_PREFIXES: [&str; 2] = ["](cid:", "](file:"];

/// Collect the lowercase hex content addresses `text` references through either marker grammar.
///
/// Deliberately a scanner and not a parser: it reads any `](cid:HEX)` / `](file:HEX)` occurrence
/// in the raw body, which is what the renderer would turn into an embed/chip. Erring toward
/// *over*-detection is the safe direction here; a false positive pins a file (it keeps
/// circulating) rather than dropping one that is still on a page.
fn scan_file_markers(text: &str, out: &mut HashSet<String>) {
    for prefix in FILE_MARKER_PREFIXES {
        let mut rest = text;
        while let Some(at) = rest.find(prefix) {
            let after = &rest[at + prefix.len()..];
            // Hex digits are ASCII, so the char count is also the byte offset.
            let end = after
                .bytes()
                .take_while(u8::is_ascii_hexdigit)
                .count()
                .min(64);
            if end > 0 && after.as_bytes().get(end) == Some(&b')') {
                out.insert(after[..end].to_ascii_lowercase());
            }
            rest = &after[end..];
        }
    }
}

/// Where a shared file is referenced across this server's documents; the "Used in" answer, and
/// the input to the never-decay rule for wiki-embedded files.
///
/// Usage is **content-addressed**: it is keyed by the file's whole-file plaintext cid, so every
/// listing of the same bytes (content dedup lists one file under several names/folders) shares
/// one usage answer. That is the honest semantic; a wiki page embeds *content*, not a listing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileUsage {
    /// Names of the live wiki pages whose body references this file, sorted. Non-empty ⇒ the
    /// file is **pinned** (see [`FileUsage::wiki_pinned`]).
    pub wiki_pages: Vec<String>,
    /// How many status-feed posts reference it.
    pub status_count: usize,
    /// How many chat messages, across every channel open on this device, reference it.
    pub chat_count: usize,
    /// How many calendar events use it: as their poster image, or through a marker in their
    /// description.
    pub event_count: usize,
}

impl FileUsage {
    /// Whether the file is embedded in at least one live wiki page, and so must **never** drop
    /// out of circulation regardless of its recorded deadline.
    pub fn wiki_pinned(&self) -> bool {
        !self.wiki_pages.is_empty()
    }
    /// Whether the file is referenced anywhere at all.
    pub fn is_empty(&self) -> bool {
        self.wiki_pages.is_empty()
            && self.status_count == 0
            && self.chat_count == 0
            && self.event_count == 0
    }
}

/// Read a content-address field: a lowercase hex string, or empty if absent, another type, or
/// not a plausible address (so a malformed entry can never send the UI fetching nonsense).
fn hex_cid_field(doc: &AutoCommit, obj: &ObjId, key: &str) -> String {
    let v = str_field(doc, obj, key);
    if !v.is_empty() && v.len() <= MAX_EVENT_IMAGE_HEX && v.bytes().all(|b| b.is_ascii_hexdigit()) {
        v.to_ascii_lowercase()
    } else {
        String::new()
    }
}

/// Read a `Bytes` scalar field (empty if absent or another type).
fn bytes_field(doc: &AutoCommit, obj: &ObjId, key: &str) -> Vec<u8> {
    match doc.get(obj, key) {
        Ok(Some((Value::Scalar(s), _))) => match s.as_ref() {
            ScalarValue::Bytes(b) => b.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// A UI-facing view of one **server** (one [`ChannelSync`] over a group). Wraps the
/// protocol with channel/message/member operations; the GUI never sees MLS or automerge.
pub struct Server<T: MeshTransport, R: CryptoRngCore> {
    sync: ChannelSync<T, R>,
    display_name: String,
    device_id: DeviceId,
    /// Per channel, the automerge change that authored each of this device's most recent
    /// messages: `(message id, change hash)`, oldest first, capped at
    /// [`MAX_TRACKED_OWN_MESSAGES`]. This is the only thing that ties a UI-visible message id to
    /// the delivery evidence in the document, and it is **not persisted**; after a restart the
    /// mapping is gone, so older messages report no delivery state at all rather than a wrong
    /// one. Only own messages are tracked; a peer's delivery is not ours to display.
    own_message_changes: HashMap<u128, VecDeque<(String, ChangeHash)>>,
    /// Per channel, the last materialized message list together with the document version
    /// ([`Server::doc_version`]) it was read at. Materializing a channel walks its whole automerge
    /// document, and the actor, the unread projection, the inbox and every UI read all ask for the
    /// same list between two changes; so the walk happens once per change rather than once per
    /// ask. Interior mutability because every reader is `&self`; the cell is never borrowed
    /// across a call that could re-enter it. Derived state; never persisted; a `RefCell` (not a
    /// lock) because a `Server` is owned by exactly one actor task.
    messages_cache: std::cell::RefCell<HashMap<u128, MaterializedChannel>>,
    /// Total order of delivery snapshots issued by this actor session. Queries and emitted events
    /// share it, so a delayed IPC query can never overwrite a newer event in the webview.
    delivery_snapshot_revision: u64,
    /// A cheap (crypto-free) content signature of the `Devices` document at the last reconcile.
    /// Re-validating that registry costs one signature check per entry, so it is rebuilt only
    /// when the document actually changed; not on every tick. `None` = never reconciled.
    devices_sig: Option<u64>,
}

/// How many of this device's most recent messages per channel carry delivery state. Bounded so a
/// long-running session cannot grow the map, and comfortably under the sync layer's per-query
/// target cap.
pub const MAX_TRACKED_OWN_MESSAGES: usize = 50;

/// Delivery state for one of this device's messages (`docs/design-delivery-states.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryState {
    /// The message id, as it appears in [`ChatMessage::id`].
    pub id: String,
    /// How many **other current-roster** members have proved they hold this message. Evidence is
    /// positive and one-sided, but the count may fall when membership changes. `0` means "no proof
    /// yet", *not* "not delivered", so a renderer must not turn it into a failure by itself.
    pub delivered: usize,
    /// How many current members claim a transport peer connected here; the same self-asserted
    /// projection that drives [`Server::online_members`]. Independent of `delivered`, which can
    /// exceed it (a member that received the message and has since disconnected still holds it).
    pub reachable: usize,
    /// Whether this node has a live peer that previously served a roster-verified signed catch-up.
    /// This is intentionally stricter than `reachable`: a self-asserted descriptor claim or bare
    /// relay/rendezvous socket cannot suppress a queued warning.
    pub any_peer: bool,
}

/// One complete delivery view plus its actor-issued order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeliverySnapshot {
    pub revision: u64,
    pub states: Vec<DeliveryState>,
}

/// A UI-facing view of one member: a short device-id **fingerprint** for display, the full
/// content-addressed identity for authorization, and whether it is **this** device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberView {
    /// Short hex fingerprint of the member's device id (first 4 bytes).
    pub fingerprint: String,
    /// Full device id. This is the value security-sensitive local policy must retain and compare.
    pub identity: String,
    /// Whether this is the local device.
    pub is_self: bool,
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Found a brand-new server with this device as the founder.
    pub fn found(
        transport: T,
        device: MlsDevice,
        rng: R,
        clock: Box<dyn Clock + Send>,
        display_name: impl Into<String>,
    ) -> Result<Self, AppError> {
        let device_id = device.device_id();
        let group = ServerGroup::create(&device)?;
        Ok(Self {
            sync: ChannelSync::new(transport, group, device, rng, clock),
            display_name: display_name.into(),
            device_id,
            own_message_changes: HashMap::new(),
            messages_cache: std::cell::RefCell::new(HashMap::new()),
            delivery_snapshot_revision: 0,
            devices_sig: None,
        })
    }

    /// Join an existing server from a pasted invite (the caller must already be
    /// transport-connected to `inviter`). Runs the authenticated MLS join handshake and
    /// adopts the transferred routing state.
    pub async fn join(
        transport: T,
        device: MlsDevice,
        rng: R,
        clock: Box<dyn Clock + Send>,
        display_name: impl Into<String>,
        inviter: PeerId,
        invite: &InviteToken,
    ) -> Result<Self, AppError> {
        let device_id = device.device_id();
        let mut connection_handoff = PreOwnerConnectionHandoff::default();
        // Bound the whole join so a never-finalizing owner (an Option-C admin invite whose owner
        // stays offline) can't wedge the joiner forever; the sync layer stays runtime-agnostic.
        let (group, routing) = tokio::time::timeout(
            std::time::Duration::from_secs(JOIN_TIMEOUT_SECS),
            request_join_tracking(
                &transport,
                &mut connection_handoff,
                inviter,
                &device,
                invite,
            ),
        )
        .await
        .map_err(|_| AppError::JoinTimeout)??;
        let mut sync = ChannelSync::new_joined(transport, group, device, rng, clock, routing);
        sync.adopt_pre_owner_connections(connection_handoff);
        // Seed the inviter as an untrusted **candidate** peer. `request_join` ran straight on the
        // transport, before this `ChannelSync` existed, so without this a brand-new member starts
        // life knowing nobody at all and cannot ask anyone for anything (PEX included) until the
        // inviter happens to send it something first. Candidate pool only; the inviter still has
        // to serve a roster-verified signed catch-up to become a trusted source.
        sync.note_candidate_peer(inviter);
        Ok(Self {
            sync,
            display_name: display_name.into(),
            device_id,
            own_message_changes: HashMap::new(),
            messages_cache: std::cell::RefCell::new(HashMap::new()),
            delivery_snapshot_revision: 0,
            devices_sig: None,
        })
    }

    /// Join through a reachable existing member when the inviter could not accept a direct dial.
    /// The helper carries only the admission handshake; the inviter named by the signed invite
    /// remains the sole Welcome authority.
    #[allow(clippy::too_many_arguments)]
    pub async fn join_via_helper(
        transport: T,
        device: MlsDevice,
        rng: R,
        clock: Box<dyn Clock + Send>,
        display_name: impl Into<String>,
        helper: PeerId,
        inviter: PeerId,
        invite: &InviteToken,
    ) -> Result<Self, AppError> {
        let device_id = device.device_id();
        let mut connection_handoff = PreOwnerConnectionHandoff::default();
        let (group, routing) = tokio::time::timeout(
            std::time::Duration::from_secs(JOIN_TIMEOUT_SECS),
            request_join_via_helper_tracking(
                &transport,
                &mut connection_handoff,
                helper,
                inviter,
                &device,
                invite,
            ),
        )
        .await
        .map_err(|_| AppError::JoinTimeout)??;
        let mut sync = ChannelSync::new_joined(transport, group, device, rng, clock, routing);
        sync.adopt_pre_owner_connections(connection_handoff);
        // Both are only candidates until a roster-verified catch-up proves them. The helper is the
        // currently reachable bootstrap; retaining the inviter as well lets discovery recover the
        // direct topology as soon as its route works.
        sync.note_candidate_peer(inviter);
        sync.note_candidate_peer(helper);
        Ok(Self {
            sync,
            display_name: display_name.into(),
            device_id,
            own_message_changes: HashMap::new(),
            messages_cache: std::cell::RefCell::new(HashMap::new()),
            delivery_snapshot_revision: 0,
            devices_sig: None,
        })
    }

    /// Complete the short-lived reply-code path without trusting the first connector. Rejected,
    /// stalled, or forged helpers are skipped until one contact returns an inviter-signed Welcome.
    #[allow(clippy::too_many_arguments)]
    pub async fn join_from_reply(
        transport: T,
        device: MlsDevice,
        rng: R,
        clock: Box<dyn Clock + Send>,
        display_name: impl Into<String>,
        first_contact: PeerId,
        inviter: PeerId,
        invite: &InviteToken,
        reply_joiner_nonce: [u8; 16],
        reply_joiner_peer: &[u8],
        expires_at_ms: u64,
    ) -> Result<(Self, PeerId), AppError> {
        Self::join_from_reply_with_handoff(
            transport,
            PreOwnerConnectionHandoff::default(),
            device,
            rng,
            clock,
            display_name,
            first_contact,
            inviter,
            invite,
            reply_joiner_nonce,
            reply_joiner_peer,
            expires_at_ms,
        )
        .await
    }

    /// Reply-code join after a bridge-side proof wait has already consumed lifecycle events.
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub async fn join_from_reply_with_handoff(
        transport: T,
        mut connection_handoff: PreOwnerConnectionHandoff,
        device: MlsDevice,
        rng: R,
        clock: Box<dyn Clock + Send>,
        display_name: impl Into<String>,
        first_contact: PeerId,
        inviter: PeerId,
        invite: &InviteToken,
        reply_joiner_nonce: [u8; 16],
        reply_joiner_peer: &[u8],
        expires_at_ms: u64,
    ) -> Result<(Self, PeerId), AppError> {
        let device_id = device.device_id();
        let (group, routing, contact) = request_join_from_reply_tracking(
            &transport,
            &mut connection_handoff,
            first_contact,
            inviter,
            &device,
            invite,
            reply_joiner_nonce,
            reply_joiner_peer,
            &*clock,
            expires_at_ms,
        )
        .await?;
        let mut sync = ChannelSync::new_joined(transport, group, device, rng, clock, routing);
        sync.adopt_pre_owner_connections(connection_handoff);
        sync.note_candidate_peer(inviter);
        sync.note_candidate_peer(contact);
        Ok((
            Self {
                sync,
                display_name: display_name.into(),
                device_id,
                own_message_changes: HashMap::new(),
                messages_cache: std::cell::RefCell::new(HashMap::new()),
                delivery_snapshot_revision: 0,
                devices_sig: None,
            },
            contact,
        ))
    }

    /// Join through only the inviter-endorsed standing switchboards carried in the outer plan.
    #[allow(clippy::too_many_arguments)]
    pub async fn join_from_switchboards(
        transport: T,
        device: MlsDevice,
        rng: R,
        clock: Box<dyn Clock + Send>,
        display_name: impl Into<String>,
        first_contact: PeerId,
        allowed_contacts: &[(PeerId, u64)],
        inviter: PeerId,
        invite: &InviteToken,
        join_plan: &[u8],
    ) -> Result<(Self, PeerId), AppError> {
        let device_id = device.device_id();
        let mut connection_handoff = PreOwnerConnectionHandoff::default();
        let join = request_join_from_switchboards_tracking(
            &transport,
            &mut connection_handoff,
            first_contact,
            allowed_contacts,
            inviter,
            &device,
            invite,
            join_plan,
            &*clock,
        );
        let (group, routing, contact) = tokio::select! {
            result = join => result?,
            _ = clock.sleep(std::time::Duration::from_secs(JOIN_TIMEOUT_SECS)) => {
                return Err(AppError::JoinTimeout);
            }
        };
        let mut sync = ChannelSync::new_joined(transport, group, device, rng, clock, routing);
        sync.adopt_pre_owner_connections(connection_handoff);
        sync.note_candidate_peer(inviter);
        sync.note_candidate_peer(contact);
        Ok((
            Self {
                sync,
                display_name: display_name.into(),
                device_id,
                own_message_changes: HashMap::new(),
                messages_cache: std::cell::RefCell::new(HashMap::new()),
                delivery_snapshot_revision: 0,
                devices_sig: None,
            },
            contact,
        ))
    }

    /// Whether a device is in this server's current MLS roster.
    pub fn contains_member_device(&self, device: &DeviceId) -> bool {
        self.sync.contains_member(device)
    }

    /// Resolve a roster device through its current self-signed peer record.
    pub fn member_transport_peer(&self, device: &DeviceId) -> Option<PeerId> {
        self.sync.member_transport_peer(device)
    }

    /// Install the explicit ephemeral capability required before this member forwards a join.
    pub fn authorize_join_helper(
        &mut self,
        joiner: PeerId,
        invite_nonce: [u8; 16],
        inviter: DeviceId,
        target: PeerId,
        expires_at_ms: u64,
    ) -> bool {
        self.sync
            .authorize_join_helper(joiner, invite_nonce, inviter, target, expires_at_ms)
    }

    /// Revoke a replaced one-time helper grant immediately.
    pub fn revoke_join_helper(&mut self, joiner: PeerId, invite_nonce: [u8; 16]) {
        self.sync.revoke_join_helper(joiner, invite_nonce);
    }

    /// Apply this device's explicit standing-switchboard consent at the protocol gate.
    pub fn set_switchboard_offered(&mut self, offered: bool) {
        self.sync.set_switchboard_offered(offered);
    }

    /// Join a server as a **companion device**, presenting a [`PerServerGrant`] from a grant
    /// bundle instead of an invite (multi-device M3).
    ///
    /// This rides the invite join's shape exactly; the caller has already connected to `contact`
    /// using the grant's `bootstrap` / `rendezvous` (which are the invite's own reach fields);
    /// with two substitutions:
    ///
    /// - the **ledger check becomes certificate verification** on the admitting owner, and
    /// - the Welcome is authenticated under the **owner's** key pinned in the grant rather than
    ///   the inviter's key pinned in an invite (only the owner ever runs the Add, and the
    ///   companion has no roster to look a key up in).
    ///
    /// `contact` need not be the owner: any member relays to the owner and forwards the Welcome,
    /// so a companion can pair against whichever member its grant could reach.
    pub async fn join_with_grant(
        transport: T,
        device: MlsDevice,
        rng: R,
        clock: Box<dyn Clock + Send>,
        display_name: impl Into<String>,
        contact: PeerId,
        grant: &PerServerGrant,
    ) -> Result<Self, AppError> {
        let device_id = device.device_id();
        let now_ms = clock.now_ms();
        let mut connection_handoff = PreOwnerConnectionHandoff::default();
        // Bounded like the invite join: an owner that never comes online to serialize the Add
        // must not wedge the device forever.
        let (group, routing) = tokio::time::timeout(
            std::time::Duration::from_secs(JOIN_TIMEOUT_SECS),
            request_device_join_tracking(
                &transport,
                &mut connection_handoff,
                contact,
                &device,
                &grant.certificate,
                &grant.owner_public_key,
                now_ms,
            ),
        )
        .await
        .map_err(|_| AppError::JoinTimeout)??;
        let mut sync = ChannelSync::new_joined(transport, group, device, rng, clock, routing);
        sync.adopt_pre_owner_connections(connection_handoff);
        // Same reasoning as the invite join: the contact that relayed this admission is the one
        // peer a fresh companion device has, so seed it as a candidate.
        sync.note_candidate_peer(contact);
        Ok(Self {
            sync,
            display_name: display_name.into(),
            device_id,
            own_message_changes: HashMap::new(),
            messages_cache: std::cell::RefCell::new(HashMap::new()),
            delivery_snapshot_revision: 0,
            devices_sig: None,
        })
    }

    /// Serialize this server's durable state for disk persistence (Phase 9f). The bytes are
    /// **secret** (signer key, group + routing secrets, plaintext content); the
    /// [`crate::store::ServerStore`] seals them under the vault key before writing.
    pub fn snapshot(&mut self) -> Result<zeroize::Zeroizing<Vec<u8>>, AppError> {
        Ok(self.sync.snapshot()?)
    }

    /// Reconstruct a server from a [`Server::snapshot`] blob plus a **fresh** transport (the
    /// caller re-dials peers, Phase 9g). The display name comes from the registry; the device
    /// id is re-derived from the restored MLS device.
    pub fn restore(
        snapshot: &[u8],
        transport: T,
        rng: R,
        clock: Box<dyn Clock + Send>,
        display_name: impl Into<String>,
    ) -> Result<Self, AppError> {
        let sync = ChannelSync::restore(snapshot, transport, rng, clock)?;
        let device_id = sync.device_id();
        Ok(Self {
            sync,
            display_name: display_name.into(),
            device_id,
            own_message_changes: HashMap::new(),
            messages_cache: std::cell::RefCell::new(HashMap::new()),
            delivery_snapshot_revision: 0,
            devices_sig: None,
        })
    }

    /// This server's MLS group id (stable across restarts); keys its on-disk blob directory.
    pub fn group_id(&self) -> Vec<u8> {
        self.sync.group_id()
    }

    /// Attach a persistent, sealing blob store (Phase 9h) so files + avatars survive restart
    /// encrypted at rest. Call right after founding/joining/restoring, before any avatar or
    /// file is added.
    pub fn set_blob_store(&mut self, blobs: Box<dyn BlobStore + Send>) {
        self.sync.set_blob_store(blobs);
    }

    /// Subscribe to membership commits (call once after founding/joining).
    pub async fn subscribe_control(&mut self) -> Result<(), AppError> {
        self.sync.subscribe_control().await?;
        Ok(())
    }

    /// Open a channel by id (creates it locally and subscribes its topic).
    pub async fn open_channel(&mut self, channel: u128) -> Result<(), AppError> {
        self.sync.open_channel(DocType::Channel, channel).await?;
        Ok(())
    }

    /// Open the shared channel directory. Actors do this once at startup, before opening every
    /// channel currently listed in it.
    pub async fn open_channel_index(&mut self) -> Result<(), AppError> {
        self.sync
            .open_channel(DocType::ChannelIndex, CHANNEL_INDEX_DOC)
            .await?;
        Ok(())
    }

    /// The server's shared channel list. Legacy servers always expose `general`, even before the
    /// first directory write has propagated.
    pub fn channels(&self) -> Vec<ChannelInfo> {
        self.sync
            .doc(DocType::ChannelIndex, CHANNEL_INDEX_DOC)
            .map(|d| read_channel_index(d.doc()))
            .unwrap_or_else(|| {
                vec![ChannelInfo {
                    id: channel_id("general"),
                    name: "general".into(),
                }]
            })
    }

    /// Add a channel to the shared directory and subscribe to its message document locally.
    /// Repeating the same normalized name is idempotent.
    pub async fn create_channel(&mut self, name: &str) -> Result<ChannelInfo, AppError> {
        let name = validate_channel_name(name)?;
        let info = ChannelInfo {
            id: channel_id(&name),
            name,
        };
        if !self.channels().iter().any(|c| c.id == info.id) {
            let key = format!("{:032x}", info.id);
            let value = info.name.clone();
            self.sync
                .post(DocType::ChannelIndex, CHANNEL_INDEX_DOC, |d| {
                    d.put(ROOT, key, value)
                })
                .await?;
        }
        self.open_channel(info.id).await?;
        Ok(info)
    }

    /// Pull the shared channel directory from one known member.
    pub async fn request_channel_index_catchup(&mut self, peer: PeerId) -> Result<usize, AppError> {
        Ok(self
            .sync
            .request_catchup(peer, DocType::ChannelIndex, CHANNEL_INDEX_DOC)
            .await?)
    }

    /// Send a chat message to a channel. The message is **authored by this device's
    /// fingerprint**; the display name + styling are resolved from the author's profile
    /// at render time (so a profile change updates all of that member's messages).
    pub async fn send_message(&mut self, channel: u128, text: &str) -> Result<(), AppError> {
        self.send_reply(channel, text, "").await
    }

    /// Send a chat message that replies to `reply_to` (the parent message's id; empty for a plain
    /// message). The pointer is advisory display metadata; it doesn't affect ordering or delivery.
    pub async fn send_reply(
        &mut self,
        channel: u128,
        text: &str,
        reply_to: &str,
    ) -> Result<(), AppError> {
        let author = self.my_fingerprint();
        let ts = self.sync.now_ms();
        let id = self.sync.random_id();
        let reply_to = reply_to.to_string();
        let change = self
            .sync
            .post(DocType::Channel, channel, |d| {
                append_message(d, &id, &author, text, ts, &reply_to)
            })
            .await?;
        self.sync
            .track_delivery_target(DocType::Channel, channel, change);
        // Remember which automerge change carried this message, so its delivery state can be
        // read back later (`delivery_snapshot`). Bounded ring: the UI only ever shows state for
        // recent own messages.
        let recent = self.own_message_changes.entry(channel).or_default();
        recent.push_back((id, change));
        while recent.len() > MAX_TRACKED_OWN_MESSAGES {
            recent.pop_front();
        }
        Ok(())
    }

    /// Edit the text of one of **your own** messages (by id) in a channel. Honest-client gating:
    /// refused if the message isn't authored by this device (a modified client could bypass it, as
    /// with all CRDT content; see THREAT-MODEL.md). A no-op edit (same text) is dropped, so the
    /// `post` always carries a real change (automerge suppresses a same-value `put`).
    ///
    /// An empty `id` is refused before any of that. It is the id every message written before ids
    /// existed carries ([`ChatMessage::id`]), so it names all of them and none of them: the author
    /// check below would pass on the caller's *own* id-less message while the write matched
    /// whichever id-less message the document holds first, putting one member's words under
    /// another member's fingerprint. Honest-client gating is the whole of what protects this
    /// document (the documented R6 residual), and a gate that passes on one message while the
    /// write lands on another is not gating anything.
    pub async fn edit_message(
        &mut self,
        channel: u128,
        id: &str,
        new_text: &str,
    ) -> Result<(), AppError> {
        if id.is_empty() {
            return Err(AppError::Invalid(
                "a message with no id cannot be addressed".into(),
            ));
        }
        let me = self.my_fingerprint();
        let Some(current) = self
            .messages(channel)
            .into_iter()
            .find(|m| m.id == id && m.author == me)
        else {
            return Err(AppError::Invalid(
                "you can only edit your own messages".into(),
            ));
        };
        if current.text == new_text {
            return Ok(()); // unchanged; don't post a redundant op
        }
        let edited = self.sync.now_ms();
        let id = id.to_string();
        let new_text = new_text.to_string();
        self.sync
            .post(DocType::Channel, channel, move |d| {
                edit_message_in_doc(d, &id, &new_text, edited).map(|_| ())
            })
            .await?;
        Ok(())
    }

    /// Delete a message (by id) from a channel: **your own**, or; if you are the owner/admin;
    /// anyone's (moderation). Honest-client gating (a modified client could post a raw delete op
    /// for any message regardless; the documented R6 residual). Errors if the message is gone or
    /// you may not delete it.
    ///
    /// An empty `id` is refused for the reason [`Server::edit_message`] gives: it names every
    /// message written before ids existed rather than one of them, so the delete would land on
    /// whichever of those the document happens to hold first.
    pub async fn delete_message(&mut self, channel: u128, id: &str) -> Result<(), AppError> {
        if id.is_empty() {
            return Err(AppError::Invalid(
                "a message with no id cannot be addressed".into(),
            ));
        }
        let me = self.my_fingerprint();
        let Some(msg) = self.messages(channel).into_iter().find(|m| m.id == id) else {
            return Err(AppError::Invalid("no such message".into()));
        };
        let moderator = matches!(self.my_role(), Role::Owner | Role::Admin);
        if msg.author != me && !moderator {
            return Err(AppError::Invalid(
                "you can only delete your own messages".into(),
            ));
        }
        let id = id.to_string();
        self.sync
            .post(DocType::Channel, channel, move |d| {
                delete_message_in_doc(d, &id).map(|_| ())
            })
            .await?;
        Ok(())
    }

    /// Toggle this member's `emoji` reaction on the message `id` in a channel (add if absent,
    /// remove if present). Anyone may react to any message. Errors if the message doesn't exist.
    ///
    /// An empty `id` is refused with the other trust-boundary checks, for the reason
    /// [`Server::edit_message`] gives: it names every message written before ids existed rather
    /// than one of them, so the reaction would land on whichever the document holds first.
    pub async fn toggle_reaction(
        &mut self,
        channel: u128,
        id: &str,
        emoji: &str,
    ) -> Result<(), AppError> {
        // Enforce the flat-key invariant at the trust boundary, before anything reaches the doc.
        if !valid_reaction_emoji(emoji) {
            return Err(AppError::Invalid("bad emoji".into()));
        }
        if id.is_empty() {
            return Err(AppError::Invalid(
                "a message with no id cannot be addressed".into(),
            ));
        }
        let me = self.my_fingerprint();
        if !self.messages(channel).iter().any(|m| m.id == id) {
            return Err(AppError::Invalid("no such message".into()));
        }
        let id = id.to_string();
        let emoji = emoji.to_string();
        self.sync
            .post(DocType::Channel, channel, move |d| {
                toggle_reaction_in_doc(d, &id, &emoji, &me).map(|_| ())
            })
            .await?;
        Ok(())
    }

    /// Pin or unpin a message (by id) in a channel. **Owner/admin only** (honest-client gating, like
    /// message deletion; the documented R6 residual). Errors if the message is gone, you may not
    /// pin, or the pin state is already as requested (no redundant op).
    ///
    /// An empty `id` is refused for the reason [`Server::edit_message`] gives: it names every
    /// message written before ids existed rather than one of them, so the pin would land on
    /// whichever the document holds first.
    pub async fn set_pin(&mut self, channel: u128, id: &str, pinned: bool) -> Result<(), AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) {
            return Err(AppError::Invalid(
                "only an owner/admin can pin messages".into(),
            ));
        }
        if id.is_empty() {
            return Err(AppError::Invalid(
                "a message with no id cannot be addressed".into(),
            ));
        }
        let Some(msg) = self.messages(channel).into_iter().find(|m| m.id == id) else {
            return Err(AppError::Invalid("no such message".into()));
        };
        if msg.pinned == pinned {
            return Ok(()); // already in the requested state; don't post a redundant op
        }
        let id = id.to_string();
        self.sync
            .post(DocType::Channel, channel, move |d| {
                set_pin_in_doc(d, &id, pinned).map(|_| ())
            })
            .await?;
        Ok(())
    }

    /// Set (or clear, with `""`) a channel's **topic**; the short description shown in its
    /// header. **Any member may set it**: channels themselves are open to create (they are
    /// name-addressed, with no registry and no gate), and a topic is channel *content* like a
    /// message, not presentation like the livery; so this is deliberately not owner/admin gated.
    /// The op is inner-signed by this device, so authorship is attributable exactly as for a
    /// message. Rejects a topic over [`MAX_CHANNEL_TOPIC_BYTES`] UTF-8 bytes; an unchanged topic
    /// is a no-op (no redundant op), like an unchanged pin.
    pub async fn set_channel_topic(&mut self, channel: u128, topic: &str) -> Result<(), AppError> {
        if topic.len() > MAX_CHANNEL_TOPIC_BYTES {
            return Err(AppError::Invalid(format!(
                "channel topic too long: {} bytes (max {MAX_CHANNEL_TOPIC_BYTES})",
                topic.len()
            )));
        }
        if self.channel_topic(channel) == topic {
            return Ok(()); // already the requested topic; don't post a redundant op
        }
        let topic = topic.to_string();
        self.sync
            .post(DocType::Channel, channel, move |d| {
                set_topic_in_doc(d, &topic)
            })
            .await?;
        Ok(())
    }

    /// A channel's current topic (empty if unset, or if the channel is not open). It lives in the
    /// channel's own document, so it replicates, persists and catches up exactly like its messages.
    pub fn channel_topic(&self, channel: u128) -> String {
        self.sync
            .doc(DocType::Channel, channel)
            .map(|d| read_topic(d.doc()))
            .unwrap_or_default()
    }

    /// Queue a shared file in a channel's **jukebox**: the channel's persistent playlist. Replies
    /// with the entry's fresh id. **Any member may**, exactly like setting the channel topic: the
    /// queue is channel *content*, not presentation, so this is deliberately not owner/admin
    /// gated. The `author` recorded in the entry is this device's own fingerprint, self-reported;
    /// real attribution is the inner signature on the op in the CRDT layer, which is what an
    /// audit reads, so a client lying in this field gains nothing.
    ///
    /// `cid` is the hex content address of an already-shared file; it is checked for shape only,
    /// not for presence, since the blob may still be in flight to this device.
    ///
    /// Rejects a `cid` that is not 1..=[`MAX_JUKEBOX_CID_CHARS`] lowercase hex digits, a blank
    /// name or one over [`MAX_JUKEBOX_NAME_BYTES`] UTF-8 bytes, and any add to a queue already
    /// holding [`MAX_JUKEBOX_ENTRIES`] entries.
    pub async fn jukebox_add(
        &mut self,
        channel: u128,
        cid: &str,
        name: &str,
    ) -> Result<String, AppError> {
        if !valid_juke_cid(cid) {
            return Err(AppError::Invalid(
                "a jukebox entry must name a file content address".into(),
            ));
        }
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::Invalid("a jukebox entry needs a name".into()));
        }
        if name.len() > MAX_JUKEBOX_NAME_BYTES {
            return Err(AppError::Invalid(format!(
                "jukebox entry name too long: {} bytes (max {MAX_JUKEBOX_NAME_BYTES})",
                name.len()
            )));
        }
        if self.jukebox(channel).len() >= MAX_JUKEBOX_ENTRIES {
            return Err(AppError::Invalid(format!(
                "the jukebox is full (max {MAX_JUKEBOX_ENTRIES} entries)"
            )));
        }
        let entry = JukeEntry {
            id: self.sync.random_id(),
            cid: cid.to_string(),
            name: name.to_string(),
            author: self.my_fingerprint(),
            added_ms: self.sync.now_ms(),
        };
        let id = entry.id.clone();
        self.sync
            .post(DocType::Channel, channel, move |d| {
                add_juke_entry_in_doc(d, &entry)
            })
            .await?;
        Ok(id)
    }

    /// Remove a jukebox entry (by id) from a channel. **Any member may**, like adding one; a
    /// shared queue that only its author could prune would strand tracks whose adder has left.
    /// Idempotent: removing an entry that is already gone is `Ok`, so a double click (or two
    /// members pruning the same track at once) is not an error.
    pub async fn jukebox_remove(&mut self, channel: u128, entry: &str) -> Result<(), AppError> {
        let entry = entry.to_string();
        self.sync
            .post(DocType::Channel, channel, move |d| {
                remove_juke_entry_in_doc(d, &entry).map(|_| ())
            })
            .await?;
        Ok(())
    }

    /// A channel's jukebox queue, sorted by queue time ascending (empty if none, or if the
    /// channel is not open). It lives in the channel's own document, so it replicates, persists
    /// and catches up exactly like its messages.
    pub fn jukebox(&self, channel: u128) -> Vec<JukeEntry> {
        self.sync
            .doc(DocType::Channel, channel)
            .map(|d| read_jukebox(d.doc()))
            .unwrap_or_default()
    }

    /// This device's short fingerprint (the key its messages + profile are stored under).
    pub fn my_fingerprint(&self) -> String {
        fingerprint(&self.device_id)
    }

    /// Open (create/subscribe) the per-server profile document. Call once after
    /// founding/joining (alongside `subscribe_control`).
    pub async fn open_profiles(&mut self) -> Result<(), AppError> {
        self.sync
            .open_channel(DocType::Profile, PROFILE_DOC)
            .await?;
        Ok(())
    }

    /// Set this member's own profile (writes the local fingerprint's entry). The avatar
    /// image (rejected if larger than [`MAX_AVATAR_BYTES`]) and the banner image (rejected if
    /// larger than [`MAX_BANNER_BYTES`]) are stored in the blob store and referenced by
    /// content address; the gossiped document carries only the CIDs.
    pub async fn set_profile(&mut self, profile: Profile) -> Result<(), AppError> {
        if profile.avatar.len() > MAX_AVATAR_BYTES {
            return Err(AppError::Invalid(format!(
                "avatar too large: {} bytes (max {MAX_AVATAR_BYTES})",
                profile.avatar.len()
            )));
        }
        if profile.banner.len() > MAX_BANNER_BYTES {
            return Err(AppError::Invalid(format!(
                "banner too large: {} bytes (max {MAX_BANNER_BYTES})",
                profile.banner.len()
            )));
        }
        let avatar_cid = if profile.avatar.is_empty() {
            Vec::new()
        } else {
            self.sync.put_blob(&profile.avatar)?.as_bytes().to_vec()
        };
        let banner_cid = if profile.banner.is_empty() {
            Vec::new()
        } else {
            self.sync.put_blob(&profile.banner)?.as_bytes().to_vec()
        };
        let fp = self.my_fingerprint();
        self.sync
            .post(DocType::Profile, PROFILE_DOC, |d| {
                write_profile(d, &fp, &profile, &avatar_cid, &banner_cid)
            })
            .await?;
        Ok(())
    }

    /// All known member profiles, keyed by device fingerprint. Each profile's avatar and
    /// banner are resolved from their content addresses against the **local** blob store;
    /// members whose image blob has not been fetched yet (see
    /// [`Server::fetch_missing_avatars`]) come back with an empty `avatar` / `banner`.
    pub fn profiles(&self) -> HashMap<String, Profile> {
        let records = self
            .sync
            .doc(DocType::Profile, PROFILE_DOC)
            .map(|d| read_profile_records(d.doc()))
            .unwrap_or_default();
        records
            .into_iter()
            .map(|(fp, r)| {
                let avatar = parse_avatar_cid(&r.avatar_cid)
                    .and_then(|cid| self.sync.get_blob(&cid))
                    .unwrap_or_default();
                let banner = parse_avatar_cid(&r.banner_cid)
                    .and_then(|cid| self.sync.get_blob(&cid))
                    .unwrap_or_default();
                (
                    fp,
                    Profile {
                        name: r.name,
                        color: r.color,
                        font: r.font,
                        effect: r.effect,
                        description: r.description,
                        bubble: r.bubble,
                        avatar,
                        banner,
                    },
                )
            })
            .collect()
    }

    /// Fetch any referenced profile image blobs (avatars **and** banners) we do not yet hold
    /// from the best known peer. Returns how many were newly fetched (so the caller can
    /// re-render). Call after the profile document changes (e.g. on join/convergence).
    /// Fetches at most [`MAX_AVATAR_FETCHES_PER_PASS`] missing blobs per call, counted across
    /// both kinds; since each fetch is a blocking mesh round-trip, this bounds how long a
    /// single pass can stall the actor, so a member churning many distinct CIDs cannot freeze
    /// peers' event loops (the remainder are picked up on subsequent ticks).
    pub async fn fetch_missing_avatars(&mut self) -> Result<usize, AppError> {
        let cids: Vec<Cid> = self
            .sync
            .doc(DocType::Profile, PROFILE_DOC)
            .map(|d| read_profile_records(d.doc()))
            .unwrap_or_default()
            .values()
            .flat_map(|r| {
                [
                    parse_avatar_cid(&r.avatar_cid),
                    parse_avatar_cid(&r.banner_cid),
                ]
            })
            .flatten()
            .collect();
        let mut fetched = 0;
        let mut attempts = 0;
        for cid in cids {
            if self.sync.has_blob(&cid) {
                continue; // cheap; does not count toward the per-pass budget
            }
            if attempts >= MAX_AVATAR_FETCHES_PER_PASS {
                break;
            }
            attempts += 1;
            if self.sync.request_blob_best(&cid).await? {
                fetched += 1;
            }
        }
        Ok(fetched)
    }

    /// Open (create/subscribe) the per-server file index document. Call once after
    /// founding/joining (alongside `subscribe_control`/`open_profiles`).
    pub async fn open_files(&mut self) -> Result<(), AppError> {
        self.sync
            .open_channel(DocType::FileIndex, FILE_INDEX_DOC)
            .await?;
        Ok(())
    }

    /// Share a file under folder `path` (`""` = root): store its bytes in the blob store and
    /// add an index entry (gossiped to the group). Returns the file's content address. Rejects
    /// files over [`MAX_FILE_BYTES`].
    ///
    /// **Content dedup**: a file's identity is its whole-file *plaintext* cid, and sealing is
    /// randomized (a fresh content key + nonce per chunk), so re-sharing identical bytes would
    /// otherwise store a second, byte-different set of ciphertext blobs for the same logical
    /// file. Instead, if that content is already listed:
    /// * same name **and** folder → nothing is added; the existing address comes back (an
    ///   idempotent re-share, e.g. a double-submit);
    /// * otherwise (the same image attached to a second wiki page, say) → a new index entry is
    ///   appended that **reuses the existing entry's ref verbatim**, so it names exactly the
    ///   already-stored chunk blobs. Nothing is sealed and no blob is written.
    ///
    /// The check is against this device's view of the index, so two devices adding the same
    /// bytes concurrently can still produce two entries; the same pre-existing situation
    /// [`delete_file`](Self::delete_file) already handles by unlisting every entry for a cid.
    /// Reuse also means the listing inherits the *first* upload's declared mime, and that a
    /// dedup against content this device has never downloaded adds a listing whose chunks are
    /// held elsewhere (re-fetchable, like any other file this device does not hold locally).
    ///
    /// **Circulation expiry**: every listing this creates is stamped
    /// `now + `[`FILE_EXPIRY_DEFAULT_MS`] (one month), adjustable afterwards per listing via
    /// [`set_file_expiry`](Self::set_file_expiry). Expiry is *per listing*, so a dedup re-list
    /// gets its **own fresh** deadline rather than inheriting the twin's; re-sharing the same
    /// bytes under a new name is a new act of sharing. Nothing enforces the deadline yet; see
    /// [`FileExpiry`].
    /// Sign the exact listing fields that a local trust policy later treats as uploader identity.
    /// The replicated op already authenticates who posted a CRDT change, but its materialized
    /// `author` scalar could otherwise be rewritten independently by a modified client.
    fn file_entry_attestation(
        &self,
        name: &str,
        author: &str,
        path: &str,
        file_ref: &[u8],
    ) -> Result<(Vec<u8>, [u8; 64]), AppError> {
        if name.len() > MAX_FILE_NAME_BYTES
            || author.len() > MAX_FILE_AUTHOR_BYTES
            || path.len() > MAX_FILE_PATH_BYTES
            || file_ref.len() > MAX_FILE_REF_BYTES
        {
            return Err(AppError::Invalid(
                "file listing metadata is too large".into(),
            ));
        }
        let public_key = self.sync.my_public_key();
        if fingerprint(&DeviceId::from_public_key_bytes(&public_key)) != author {
            return Err(AppError::Invalid(
                "local file author does not match this device key".into(),
            ));
        }
        let payload =
            file_entry_attestation_payload(&self.sync.group_id(), name, author, path, file_ref);
        let signature = self.sync.sign_blob(&payload)?;
        Ok((public_key, signature))
    }

    pub async fn add_file(
        &mut self,
        name: &str,
        mime: &str,
        path: &str,
        bytes: &[u8],
    ) -> Result<Cid, AppError> {
        self.add_file_with_progress(name, mime, path, bytes, None)
            .await
    }

    /// As [`add_file`](Self::add_file), but reports completed local upload work as
    /// `(steps_done, steps_total)`. Each sealed/stored chunk is one step and publishing the file
    /// index entry is the final step, so `done == total` means the file is actually visible to the
    /// group rather than merely copied into local storage.
    pub async fn add_file_with_progress(
        &mut self,
        name: &str,
        mime: &str,
        path: &str,
        bytes: &[u8],
        progress: Option<&tokio::sync::mpsc::Sender<(usize, usize)>>,
    ) -> Result<Cid, AppError> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(AppError::Invalid(format!(
                "file too large: {} bytes (max {MAX_FILE_BYTES})",
                bytes.len()
            )));
        }
        if !self.sync.has_file_key() {
            return Err(AppError::Invalid(
                "no group file key yet (still joining?)".into(),
            ));
        }
        // Chunked transfer: split into CHUNK_BYTES pieces, seal each under the group file-wrap key
        // (9h; a fresh per-chunk content key) and store the ciphertext blob, then describe the
        // file by a manifest (the whole-file plaintext cid + the ordered chunk FileRefs). The
        // (encrypted) index carries the manifest; the blob fetch caps only per-chunk size. The
        // file's identity is its whole-file plaintext cid.
        let author = self.my_fingerprint();
        let folder = normalize_path(path);
        if name.len() > MAX_FILE_NAME_BYTES
            || mime.len() > MAX_FILE_MIME_BYTES
            || folder.len() > MAX_FILE_PATH_BYTES
        {
            return Err(AppError::Invalid(
                "file listing metadata is too large".into(),
            ));
        }
        // Clock-injected (never ambient): the default one-month circulation deadline.
        let expires = FileExpiry::At(self.sync.now_ms().saturating_add(FILE_EXPIRY_DEFAULT_MS));
        let plaintext_cid = Cid::of(bytes);
        let index_rows = self.file_index_row_count();
        // Dedup on the plaintext cid against the live index (a deleted entry is removed from the
        // list, so only still-shared files match; re-storing after a delete is harmless anyway).
        let listed = self.files();
        let twins: Vec<&FileEntry> = listed
            .iter()
            .filter(|e| e.cid.as_slice() == plaintext_cid.as_bytes())
            .collect();
        if let Some(twin) = twins.first() {
            if let Some(p) = progress {
                let _ = p.send((0, 1)).await;
            }
            if twins.iter().any(|e| e.name == name && e.path == folder) {
                if let Some(p) = progress {
                    let _ = p.send((1, 1)).await;
                }
                return Ok(plaintext_cid); // already shared under this exact name + folder
            }
            if index_rows >= MAX_FILE_ENTRIES {
                return Err(AppError::Invalid(format!(
                    "file index has reached its {MAX_FILE_ENTRIES}-entry limit"
                )));
            }
            // Same content, new name/folder: list it again against the SAME sealed blobs; but
            // with its own fresh deadline, not the twin's.
            let ref_bytes = twin.file_ref.clone();
            let (public_key, signature) =
                self.file_entry_attestation(name, &author, &folder, &ref_bytes)?;
            self.sync
                .post(DocType::FileIndex, FILE_INDEX_DOC, |d| {
                    write_file_entry(
                        d,
                        name,
                        &author,
                        &folder,
                        &ref_bytes,
                        expires,
                        Some((&public_key, &signature)),
                    )
                })
                .await?;
            if let Some(p) = progress {
                let _ = p.send((1, 1)).await;
            }
            return Ok(plaintext_cid);
        }
        if index_rows >= MAX_FILE_ENTRIES {
            return Err(AppError::Invalid(format!(
                "file index has reached its {MAX_FILE_ENTRIES}-entry limit"
            )));
        }
        let chunk_count = bytes.len().max(1).div_ceil(CHUNK_BYTES);
        let total_steps = chunk_count + 1; // the final step publishes the index entry
        if let Some(p) = progress {
            let _ = p.send((0, total_steps)).await;
        }
        let mut chunks = Vec::new();
        for (i, chunk) in bytes.chunks(CHUNK_BYTES).enumerate() {
            chunks.push(self.seal_upload_chunk(chunk, mime)?);
            if let Some(p) = progress {
                let _ = p.send((i + 1, total_steps)).await;
            }
        }
        if chunks.is_empty() {
            // An empty file is still one (empty) chunk, so the manifest always has >= 1.
            chunks.push(self.seal_upload_chunk(&[], mime)?);
            if let Some(p) = progress {
                let _ = p.send((1, total_steps)).await;
            }
        }
        // The dedup fast path above already returned, so this publishes the fresh manifest.
        let cid = self
            .publish_upload(name, mime, path, plaintext_cid, bytes.len() as u64, chunks)
            .await?;
        if let Some(p) = progress {
            let _ = p.send((total_steps, total_steps)).await;
        }
        Ok(cid)
    }

    /// Seal + store **one** chunk of a *streamed* upload, returning the [`FileRef`] that the final
    /// [`publish_upload`](Self::publish_upload) records in the file's manifest.
    ///
    /// This is the upload twin of [`fetch_file_chunk`](Self::fetch_file_chunk). Sealing a whole
    /// file in one call occupies the server actor for the entire upload: nothing else that server
    /// does (inbound sync, presence, every other UI command) runs meanwhile, and the caller has to
    /// hold every byte at once as well. Driving one of these per chunk keeps the longest
    /// uninterruptible unit of upload work at one chunk, and lets a caller stream a file it never
    /// holds whole.
    ///
    /// `bytes` must be exactly [`CHUNK_BYTES`] for every chunk except the last: the media reader
    /// maps a byte offset to a chunk index by dividing, so a manifest's chunks are uniformly sized
    /// by construction. The caller owns that ordering; this call is deliberately stateless.
    ///
    /// The chunk lands in the blob store's **staging** area, not the store proper. An upload's
    /// chunks are not held content until its manifest names them, and staging is what makes that
    /// distinction survive a crash: written straight into the store, an upload that never finished
    /// left blobs indistinguishable from real ones, which no sweep could safely reclaim.
    /// [`publish_upload`](Self::publish_upload) promotes them; anything else collects them.
    pub fn seal_upload_chunk(&mut self, bytes: &[u8], mime: &str) -> Result<FileRef, AppError> {
        if bytes.len() > CHUNK_BYTES {
            return Err(AppError::Invalid(format!(
                "upload chunk too large: {} bytes (max {CHUNK_BYTES})",
                bytes.len()
            )));
        }
        if !self.sync.has_file_key() {
            return Err(AppError::Invalid(
                "no group file key yet (still joining?)".into(),
            ));
        }
        let (file_ref, ciphertext) = self.sync.seal_file(bytes, mime)?;
        self.sync.put_staged_blob(&ciphertext)?;
        Ok(file_ref)
    }

    /// Drop every staged blob this server holds, returning how many went.
    ///
    /// Run once at startup. Staging holds the chunks of uploads that were in flight, and the only
    /// thing that knew what any of them was for was the process that staged it: nothing can claim
    /// them now, so they are unambiguously garbage. This is what closes the crash window that a
    /// purely in-memory record of pending uploads could not.
    pub fn clear_staged_uploads(&mut self) -> usize {
        self.sync.clear_staged_blobs().unwrap_or(0)
    }

    /// Publish the file-index entry for an upload whose chunks are already sealed and stored,
    /// making it visible to the group. `plaintext_cid` is the address of the **whole** file (a
    /// streaming caller accumulates it with [`CidHasher`](catcoms_storage::CidHasher)) and becomes
    /// the file's identity, exactly as in [`add_file`](Self::add_file).
    ///
    /// Content dedup works the same way, with one difference forced by streaming: a streamed
    /// upload only learns its whole-file address after its last chunk, so the twin check lands
    /// *after* sealing rather than before it. When a twin is found the new listing reuses the
    /// twin's ref (or is skipped entirely, for an identical name + folder) and the chunk blobs
    /// this upload just wrote are garbage-collected, so a dedup still ends with exactly one sealed
    /// copy of the content rather than a second, byte-different one.
    pub async fn publish_upload(
        &mut self,
        name: &str,
        mime: &str,
        path: &str,
        plaintext_cid: Cid,
        total_size: u64,
        chunks: Vec<FileRef>,
    ) -> Result<Cid, AppError> {
        if total_size > MAX_FILE_BYTES as u64 {
            return Err(AppError::Invalid(format!(
                "file too large: {total_size} bytes (max {MAX_FILE_BYTES})"
            )));
        }
        if chunks.is_empty() {
            return Err(AppError::Invalid("an upload has at least one chunk".into()));
        }
        if !self.sync.has_file_key() {
            return Err(AppError::Invalid(
                "no group file key yet (still joining?)".into(),
            ));
        }
        let author = self.my_fingerprint();
        let folder = normalize_path(path);
        if name.len() > MAX_FILE_NAME_BYTES
            || mime.len() > MAX_FILE_MIME_BYTES
            || folder.len() > MAX_FILE_PATH_BYTES
        {
            self.discard_upload_chunks(&chunks);
            return Err(AppError::Invalid(
                "file listing metadata is too large".into(),
            ));
        }
        // Clock-injected (never ambient): the default one-month circulation deadline.
        let expires = FileExpiry::At(self.sync.now_ms().saturating_add(FILE_EXPIRY_DEFAULT_MS));
        let index_rows = self.file_index_row_count();
        let listed = self.files();
        let twin = listed
            .iter()
            .find(|e| e.cid.as_slice() == plaintext_cid.as_bytes())
            .map(|e| {
                (
                    e.file_ref.clone(),
                    listed
                        .iter()
                        .any(|o| o.cid == e.cid && o.name == name && o.path == folder),
                )
            });
        if let Some((twin_ref, already_here)) = twin {
            self.discard_upload_chunks(&chunks);
            if already_here {
                return Ok(plaintext_cid); // already shared under this exact name + folder
            }
            if index_rows >= MAX_FILE_ENTRIES {
                return Err(AppError::Invalid(format!(
                    "file index has reached its {MAX_FILE_ENTRIES}-entry limit"
                )));
            }
            // Same content, new name/folder: list it again against the SAME sealed blobs; but
            // with its own fresh deadline, not the twin's.
            let (public_key, signature) =
                self.file_entry_attestation(name, &author, &folder, &twin_ref)?;
            self.sync
                .post(DocType::FileIndex, FILE_INDEX_DOC, |d| {
                    write_file_entry(
                        d,
                        name,
                        &author,
                        &folder,
                        &twin_ref,
                        expires,
                        Some((&public_key, &signature)),
                    )
                })
                .await?;
            return Ok(plaintext_cid);
        }
        if index_rows >= MAX_FILE_ENTRIES {
            self.discard_upload_chunks(&chunks);
            return Err(AppError::Invalid(format!(
                "file index has reached its {MAX_FILE_ENTRIES}-entry limit"
            )));
        }
        let manifest = FileManifest {
            plaintext_cid,
            total_size,
            mime: mime.to_string(),
            chunks,
        };
        // Never publish a listing this node's own reader would reject. A streamed upload assembles
        // its chunks across many commands, so this is also where a caller that mis-sliced the file
        // is caught, rather than every peer discovering it at download time.
        manifest
            .validate_layout()
            .map_err(|_| AppError::Invalid("the upload's chunk layout is invalid".into()))?;
        // Promote before posting, never after. The window between the two is the only one left in
        // which a crash strands anything, and this ordering makes that window strand *orphans*
        // (blobs nothing names, harmless) rather than a *published listing whose chunks are still
        // in staging*, which the next startup sweep would delete out from under the only device
        // that holds them.
        for chunk in &manifest.chunks {
            self.sync.promote_staged_blob(&chunk.ciphertext_cid)?;
        }
        let ref_bytes = manifest.encode();
        let (public_key, signature) =
            self.file_entry_attestation(name, &author, &folder, &ref_bytes)?;
        self.sync
            .post(DocType::FileIndex, FILE_INDEX_DOC, |d| {
                write_file_entry(
                    d,
                    name,
                    &author,
                    &folder,
                    &ref_bytes,
                    expires,
                    Some((&public_key, &signature)),
                )
            })
            .await?;
        Ok(plaintext_cid)
    }

    /// Drop the chunk blobs of an upload that will never be published: an abandoned or failed
    /// streamed upload, or one whose content the dedup in
    /// [`publish_upload`](Self::publish_upload) found already held.
    ///
    /// Safe by construction, not by check: an unpublished upload's chunks live in the staging
    /// area, which nothing references, so dropping them cannot take storage a shared file depends
    /// on. (The held-content path is still swept dedup-safely, for the case of a caller handing
    /// back refs that were already promoted.)
    pub fn discard_upload_chunks(&mut self, chunks: &[FileRef]) {
        let mut unstaged: Vec<&FileRef> = Vec::new();
        for chunk in chunks {
            match self.sync.drop_staged_blob(&chunk.ciphertext_cid) {
                Ok(true) => {}             // staged: gone, and it was never held
                _ => unstaged.push(chunk), // already promoted, or never written
            }
        }
        if unstaged.is_empty() {
            return;
        }
        let live: HashSet<Cid> = self
            .files()
            .iter()
            .filter_map(|e| FileManifest::decode_or_legacy(&e.file_ref).ok())
            .flat_map(|m| m.chunks.into_iter().map(|c| c.ciphertext_cid))
            .collect();
        for chunk in unstaged {
            if !live.contains(&chunk.ciphertext_cid) {
                let _ = self.sync.delete_blob(&chunk.ciphertext_cid);
            }
        }
    }

    /// The shared files listed in the index (metadata only; bytes are fetched on download).
    pub fn files(&self) -> Vec<FileEntry> {
        let group_id = self.sync.group_id();
        self.sync
            .doc(DocType::FileIndex, FILE_INDEX_DOC)
            .map(|d| read_file_entries(d.doc(), &group_id))
            .unwrap_or_default()
    }

    /// Raw replicated row count, including malformed rows that fail materialization. Publication
    /// must use this rather than `files().len()`: otherwise hostile invalid rows can push a newly
    /// published honest entry beyond the reader cap and make a successful upload invisible.
    fn file_index_row_count(&self) -> usize {
        self.sync
            .doc(DocType::FileIndex, FILE_INDEX_DOC)
            .map(|encrypted| raw_file_index_row_count(encrypted.doc()))
            .unwrap_or_default()
    }

    /// Download + decrypt a shared file by its whole-file plaintext content address. Fetches each
    /// chunk's ciphertext blob from the best known peer if not already held, opens it under the
    /// group file-wrap key (9h), reassembles in order, and verifies the whole-file plaintext cid.
    /// Returns a *precise* error (naming the failing chunk) so the distinct failures aren't
    /// conflated: the file isn't listed · the ref is corrupt · a chunk is held but unreadable ·
    /// no peer has a chunk yet · a chunk couldn't be decrypted · the reassembly failed its check.
    pub async fn download_file(&mut self, cid: &Cid) -> Result<Vec<u8>, AppError> {
        self.download_file_with_progress(cid, None).await
    }

    /// As [`download_file`](Self::download_file), but reports per-chunk progress over `progress` as
    /// `(chunks_done, chunks_total)`; `(0, total)` first so the UI shows 0% immediately, then
    /// `(i+1, total)` after each chunk. The actor wires this to a `DownloadProgress` event so a
    /// large multi-chunk download shows a progress bar.
    pub async fn download_file_with_progress(
        &mut self,
        cid: &Cid,
        progress: Option<&tokio::sync::mpsc::Sender<(usize, usize, Option<String>)>>,
    ) -> Result<Vec<u8>, AppError> {
        let Some(entry) = self
            .files()
            .into_iter()
            .find(|e| e.cid.as_slice() == &cid.as_bytes()[..])
        else {
            return Err(AppError::Invalid(
                "no such file in this server's index".into(),
            ));
        };
        let manifest = FileManifest::decode_or_legacy(&entry.file_ref)
            .map_err(|_| AppError::Invalid("corrupt file reference".into()))?;
        // `total_size` is attacker-controlled (a member authors the manifest); reject an absurd
        // value BEFORE pre-allocating, so a hostile listing can't OOM the downloader's actor.
        if manifest.total_size > MAX_FILE_BYTES as u64 {
            return Err(AppError::Invalid(
                "file reference declares an implausible size".into(),
            ));
        }
        let total = manifest.chunks.len();
        if let Some(p) = progress {
            let _ = p.send((0, total, None)).await;
        }
        let mut out = Vec::with_capacity(manifest.total_size as usize);
        for (i, chunk_ref) in manifest.chunks.iter().enumerate() {
            let (chunk, provider) = self.fetch_and_open_chunk(chunk_ref, i).await?;
            out.extend_from_slice(&chunk);
            if let Some(p) = progress {
                let _ = p.send((i + 1, total, provider)).await;
            }
        }
        // End-to-end integrity: the reassembled plaintext must hash to the manifest's identity.
        if Cid::of(&out) != manifest.plaintext_cid {
            return Err(AppError::Invalid(
                "the reassembled file failed its integrity check".into(),
            ));
        }
        Ok(out)
    }

    /// Fetch (if not readable) + decrypt one chunk, returning its plaintext bytes and the signed
    /// provider that served it. The single exclusive-state need on the fetch path is `blobs.put`;
    /// everything else is read-only. Shared by the all-in-one download and the per-chunk path.
    async fn fetch_and_open_chunk(
        &mut self,
        chunk_ref: &FileRef,
        idx: usize,
    ) -> Result<(Vec<u8>, Option<String>), AppError> {
        self.fetch_and_open_chunk_cancellable(chunk_ref, idx, None)
            .await
    }

    /// Cancellable chunk path for bounded whole-file reads. Local blobs complete normally; a
    /// network miss carries cancellation/accounting into the transport actor that owns it.
    async fn fetch_and_open_chunk_cancellable(
        &mut self,
        chunk_ref: &FileRef,
        idx: usize,
        cancellation: Option<RequestCancellation>,
    ) -> Result<(Vec<u8>, Option<String>), AppError> {
        let ccid = chunk_ref.ciphertext_cid;
        // Path existence is not enough: a corrupt sealed record must be fetched again. Capture
        // the signed responder so repair remains attributable in the transfer UI.
        let provider = if self.sync.get_blob(&ccid).is_none() {
            match cancellation {
                Some(cancellation) => {
                    self.sync
                        .request_blob_best_provider_cancellable(&ccid, cancellation)
                        .await?
                }
                None => self.sync.request_blob_best_provider(&ccid).await?,
            }
        } else {
            None
        };
        let Some(ciphertext) = self.sync.get_blob(&ccid) else {
            return Err(if self.sync.has_blob(&ccid) {
                AppError::Invalid(format!(
                    "chunk {idx} is stored on this device but could not be read (it may be corrupted)"
                ))
            } else {
                AppError::Invalid(format!(
                    "file not available yet; no connected peer has chunk {idx}"
                ))
            });
        };
        let chunk = self
            .sync
            .open_file(&ciphertext, chunk_ref)
            .map_err(|e| AppError::Invalid(format!("chunk {idx} could not be decrypted: {e}")))?;
        Ok((chunk, provider))
    }

    /// The download plan for a listed file: `(total chunks, total plaintext size)`. `None` if the
    /// file isn't listed, its reference is corrupt, or it declares an implausible size. The
    /// orchestrator fetches the chunks one per command (see [`Server::fetch_file_chunk`]) so the
    /// actor stays responsive between chunks instead of blocking for the whole download.
    pub fn file_download_plan(&self, cid: &Cid) -> Option<(usize, u64)> {
        let entry = self
            .files()
            .into_iter()
            .find(|e| e.cid.as_slice() == &cid.as_bytes()[..])?;
        let manifest = FileManifest::decode_or_legacy(&entry.file_ref).ok()?;
        if manifest.total_size > MAX_FILE_BYTES as u64 {
            return None;
        }
        Some((manifest.chunks.len(), manifest.total_size))
    }

    /// The plaintext size and declared type of a listed file, from the index alone.
    ///
    /// The media protocol needs both before it can answer anything: a suffix range, a
    /// past-the-end probe and the `Content-Range` total are all relative to the size, and the
    /// type decides what the player is even willing to decode. It used to learn them by reading
    /// **chunk 0** of the file on every single response, which is a whole 8 MiB fetch-and-open on
    /// the single-threaded actor to discover two numbers the manifest already carries. With a
    /// two-chunk plaintext cache that read also evicted the chunk being played, so a player
    /// walking a long track re-read chunk 0, dropped the chunk it was playing, and re-read that
    /// too: a file every byte of which was already on this disk could still stall the deck. This
    /// touches no blob, no disk and no network.
    pub fn file_head(&self, cid: &Cid) -> Option<FileMediaHead> {
        let (entry, manifest, manifest_version) = self.unique_media_entry(cid)?;
        Some(FileMediaHead {
            total_size: manifest.total_size,
            mime: entry.mime,
            manifest_version,
        })
    }

    /// Resolve one CID to exactly one current chunk manifest.
    ///
    /// A replicated index may contain several legitimate names/paths for identical content, but
    /// every such row must carry byte-identical `file_ref` bytes. A malicious member can otherwise
    /// repeat a benign plaintext CID while naming different encrypted chunks; treating the CID as
    /// the cache identity would authorize those replacement bytes under a stale MIME decision.
    fn unique_media_entry(&self, cid: &Cid) -> Option<(FileEntry, FileManifest, [u8; 32])> {
        let mut entries = self
            .files()
            .into_iter()
            .filter(|entry| entry.cid.as_slice() == &cid.as_bytes()[..]);
        let entry = entries.next()?;
        let manifest_version = file_manifest_version(&entry.file_ref);
        if entries.any(|candidate| file_manifest_version(&candidate.file_ref) != manifest_version) {
            return None;
        }
        let manifest = FileManifest::decode_or_legacy(&entry.file_ref).ok()?;
        // The same guard `read_file_range` applies: a member authors the manifest, so an absurd
        // declared size must not become a `Content-Range` the player then chases.
        if manifest.total_size > MAX_FILE_BYTES as u64 {
            return None;
        }
        Some((entry, manifest, manifest_version))
    }

    /// Read a byte range of a listed file's plaintext.
    ///
    /// This is the media path: a player asks for the window it is about to show rather than the
    /// whole file, so playback can start on the first chunk and a seek costs one chunk instead of
    /// a re-download. It goes through exactly the same local-first fetch as a download
    /// ([`Server::fetch_and_open_chunk`]), so a chunk already in the vault never touches the
    /// network and a corrupt one is re-fetched, and every chunk is AEAD-opened before it is
    /// served. What it does *not* do is the whole-file content-address check a download ends
    /// with: that check needs every byte, and the point here is to not have every byte. Chunk
    /// authentication is what stands in for it, which is why a range is only ever assembled out
    /// of opened chunks.
    pub async fn read_file_range(
        &mut self,
        cid: &Cid,
        expected_manifest_version: [u8; 32],
        start: u64,
        max_len: usize,
    ) -> Result<FileRange, AppError> {
        let Some((entry, manifest, manifest_version)) = self.unique_media_entry(cid) else {
            return Err(AppError::Invalid(
                "no unique file manifest in this server's index".into(),
            ));
        };
        if manifest_version != expected_manifest_version {
            return Err(AppError::Invalid(
                "file manifest changed after media authorization".into(),
            ));
        }
        let mime = entry.mime.clone();
        let total_size = manifest.total_size;
        if total_size > MAX_FILE_BYTES as u64 {
            return Err(AppError::Invalid(
                "file declares an implausible size".into(),
            ));
        }
        if start >= total_size {
            // A player probing past the end is normal, not an error; an empty tail says so.
            return Ok(FileRange {
                bytes: Vec::new(),
                total_size,
                mime,
                provider: None,
            });
        }
        let end = start.saturating_add(max_len as u64).min(total_size);
        let first = (start / CHUNK_BYTES as u64) as usize;
        let last = ((end - 1) / CHUNK_BYTES as u64) as usize;
        let mut buf = Vec::with_capacity((end - start) as usize);
        let mut provider = None;
        for idx in first..=last {
            let Some(chunk_ref) = manifest.chunks.get(idx).cloned() else {
                return Err(AppError::Invalid(format!("chunk {idx} is out of range")));
            };
            let (chunk, from) = self.fetch_and_open_chunk(&chunk_ref, idx).await?;
            // Every chunk but the last is exactly CHUNK_BYTES (`bytes.chunks(CHUNK_BYTES)` on the
            // way in), which is the whole basis for turning a byte offset into a chunk index
            // without reading everything before it. Check it rather than trust it: if the two ever
            // drift, silently serving misaligned bytes would corrupt playback in a way that looks
            // like a codec bug.
            if idx != manifest.chunks.len() - 1 && chunk.len() != CHUNK_BYTES {
                return Err(AppError::Invalid(format!(
                    "chunk {idx} is {} bytes, expected {CHUNK_BYTES}",
                    chunk.len()
                )));
            }
            provider = provider.or(from);
            let base = idx as u64 * CHUNK_BYTES as u64;
            let lo = start.saturating_sub(base).min(chunk.len() as u64) as usize;
            let hi = (end - base).min(chunk.len() as u64) as usize;
            buf.extend_from_slice(&chunk[lo..hi]);
        }
        Ok(FileRange {
            bytes: buf,
            total_size,
            mime,
            provider,
        })
    }

    /// Fetch + decrypt a single chunk (`idx`) of a listed file, returning its plaintext bytes + the
    /// provider that served it. One chunk per call so the actor can interleave other work between
    /// chunks; the orchestrator reassembles and verifies the whole-file content address.
    pub async fn fetch_file_chunk(
        &mut self,
        cid: &Cid,
        idx: usize,
    ) -> Result<(Vec<u8>, Option<String>), AppError> {
        self.fetch_file_chunk_cancellable(cid, idx, None).await
    }

    /// Cancellable [`Self::fetch_file_chunk`] whose network request retains caller accounting
    /// until the production transport reports that exact request responded, failed, or timed out.
    pub async fn fetch_file_chunk_cancellable(
        &mut self,
        cid: &Cid,
        idx: usize,
        cancellation: Option<RequestCancellation>,
    ) -> Result<(Vec<u8>, Option<String>), AppError> {
        // Re-resolve the manifest each call (cheap vs. a chunk fetch). Deliberate: it keeps the
        // per-chunk path current with the index, so a file unlisted mid-download fails cleanly here
        // rather than serving from a stale manifest.
        let Some(entry) = self
            .files()
            .into_iter()
            .find(|e| e.cid.as_slice() == &cid.as_bytes()[..])
        else {
            return Err(AppError::Invalid(
                "no such file in this server's index".into(),
            ));
        };
        let manifest = FileManifest::decode_or_legacy(&entry.file_ref)
            .map_err(|_| AppError::Invalid("corrupt file reference".into()))?;
        let Some(chunk_ref) = manifest.chunks.get(idx).cloned() else {
            return Err(AppError::Invalid(format!("chunk {idx} is out of range")));
        };
        self.fetch_and_open_chunk_cancellable(&chunk_ref, idx, cancellation)
            .await
    }

    /// Whether this device already holds **all** of the file's chunk blobs locally; i.e. it can
    /// be opened/previewed without a network fetch. (A listed file whose chunks aren't all held
    /// yet is still downloadable from peers that have them.)
    pub fn file_available(&self, cid: &Cid) -> bool {
        let Some(entry) = self
            .files()
            .into_iter()
            .find(|e| e.cid.as_slice() == &cid.as_bytes()[..])
        else {
            return false;
        };
        let (held, total) = self.chunk_holding(&entry);
        held == total // vacuously true for a (degenerate) zero-chunk file, matching `all()`
    }

    /// How many of a listed file's chunks this device holds locally, as `(held, total)`. A pure
    /// local blob-store check (zero network cost). A corrupt/undecodable `file_ref` yields `(0, 0)`.
    fn chunk_holding(&self, entry: &FileEntry) -> (u32, u32) {
        match FileManifest::decode_or_legacy(&entry.file_ref) {
            Ok(manifest) => {
                let total = manifest.chunks.len() as u32;
                let held = manifest
                    .chunks
                    .iter()
                    .filter(|c| self.sync.has_blob(&c.ciphertext_cid))
                    .count() as u32;
                (held, total)
            }
            Err(_) => (0, 0),
        }
    }

    /// The shared file list with per-file local-availability counts and a cheap "any peer
    /// reachable" flag, for the file browser's availability indicator. Zero network cost; purely
    /// local blob-store + in-memory peer-set checks. See [`FilesView`].
    pub fn files_view(&self) -> FilesView {
        self.files_view_from_entries(self.files())
    }

    fn files_view_from_entries(&self, entries: Vec<FileEntry>) -> FilesView {
        let files = entries
            .into_iter()
            .map(|entry| {
                let (held_chunks, total_chunks) = self.chunk_holding(&entry);
                let manifest_version = file_manifest_version(&entry.file_ref);
                FileListing {
                    entry,
                    held_chunks,
                    total_chunks,
                    manifest_version,
                }
            })
            .collect();
        FilesView {
            files,
            has_peers: self.has_fetch_peers(),
        }
    }

    /// Verify every unique file chunk referenced by the shared index. This performs local I/O and
    /// cryptographic checks but no network requests and no mutation.
    pub fn storage_health(&self) -> StorageHealth {
        let files = self.files();
        self.storage_health_for_entries(&files)
    }

    /// Capture the exact file rows and their cryptographic local-health verdict in one actor turn.
    pub fn storage_snapshot(&self) -> StorageSnapshot {
        let files = self.files();
        StorageSnapshot {
            health: self.storage_health_for_entries(&files),
            files: self.files_view_from_entries(files),
        }
    }

    fn storage_health_for_entries(&self, files: &[FileEntry]) -> StorageHealth {
        self.storage_health_for_entries_with(files, |ciphertext, file_ref| {
            self.sync.open_file(ciphertext, file_ref).is_ok()
        })
    }

    fn storage_health_for_entries_with<F>(
        &self,
        files: &[FileEntry],
        mut open_file: F,
    ) -> StorageHealth
    where
        F: FnMut(&[u8], &FileRef) -> bool,
    {
        // Aggregate byte counts remain deduplicated by ciphertext CID, but authentication work and
        // whole-file verdicts are keyed by the exact FileRef. Those identities cannot be merged:
        // the same sealed bytes paired with a different wrapped key are a different claim. More
        // than the small exact-ref cap is treated as ambiguity, not multiplied crypto work.
        let (refs, file_refs, invalid_manifests) = storage_ref_index(files);

        let mut health = StorageHealth {
            listed_files: files.len(),
            referenced_chunks: refs.len(),
            invalid_manifests,
            has_peers: self.has_fetch_peers(),
            ..StorageHealth::default()
        };
        let mut verified_refs = HashSet::new();
        for (cid, exact_refs) in refs {
            match exact_refs {
                BoundedFileRefs::Ambiguous => health.unreadable_chunks += 1,
                BoundedFileRefs::Exact(exact_refs) => match self.sync.get_blob(&cid) {
                    Some(ciphertext) => {
                        let mut all_refs_verified = true;
                        for (version, file_ref) in exact_refs {
                            if open_file(&ciphertext, &file_ref) {
                                verified_refs.insert(version);
                            } else {
                                all_refs_verified = false;
                            }
                        }
                        if all_refs_verified {
                            health.verified_chunks += 1;
                            health.verified_bytes = health
                                .verified_bytes
                                .saturating_add(ciphertext.len() as u64);
                        } else {
                            health.unreadable_chunks += 1;
                        }
                    }
                    None if self.sync.has_blob(&cid) => health.unreadable_chunks += 1,
                    None => health.missing_chunks += 1,
                },
            }
        }
        health.verified_manifest_versions = file_refs
            .into_iter()
            .filter_map(|(manifest, refs)| {
                (!refs.is_empty() && refs.iter().all(|version| verified_refs.contains(version)))
                    .then_some(manifest)
            })
            .collect();
        health
    }

    /// Re-fetch every repairable missing or unreadable referenced chunk from the best connected
    /// member, then verify the whole set again. Contradictory over-cap exact references remain
    /// visibly unreadable without a fetch: content addressing would return the same ciphertext and
    /// cannot reconcile different wrapped keys. The sync/storage path authenticates the responder
    /// and CID before replacing an unreadable record; no unreferenced blob is garbage-collected.
    pub async fn repair_storage(&mut self) -> Result<StorageRepair, AppError> {
        let before = self.storage_health();
        let mut candidates = HashSet::new();
        let files = self.files();
        let (refs, _, _) = storage_ref_index(&files);
        for (cid, exact_refs) in refs {
            let BoundedFileRefs::Exact(exact_refs) = exact_refs else {
                // Fetching the same content-addressed ciphertext cannot reconcile contradictory
                // wrapped keys. Leave this CID visibly unreadable without wasting peer traffic.
                continue;
            };
            let readable = self
                .sync
                .get_blob(&cid)
                .map(|bytes| {
                    exact_refs
                        .values()
                        .all(|chunk| self.sync.open_file(&bytes, chunk).is_ok())
                })
                .unwrap_or(false);
            if !readable {
                candidates.insert(cid);
            }
        }
        for cid in &candidates {
            let _ = self.sync.request_blob_best(cid).await?;
        }
        let health = self.storage_health();
        let bad_before = before.missing_chunks + before.unreadable_chunks;
        let bad_after = health.missing_chunks + health.unreadable_chunks;
        Ok(StorageRepair {
            attempted_chunks: candidates.len(),
            recovered_chunks: bad_before.saturating_sub(bad_after),
            health,
        })
    }

    /// Whether ≥1 live peer previously served a roster-verified, request-bound signed catch-up;
    /// a cheap proxy for "a missing chunk could be fetched". A self-asserted descriptor claim or
    /// bare relay/rendezvous socket is deliberately excluded: neither proves that connection can
    /// serve group data. This does not prove the peer holds a particular file, and every fetch is
    /// authenticated again. Zero network cost (an in-memory session-state check).
    pub fn has_fetch_peers(&self) -> bool {
        self.sync.has_proven_connected_member_peer()
    }

    /// Current members whose self-asserted peer id has a live connection here, for local route
    /// presence diagnostics. This is neither proof that the member controls that transport nor
    /// that the person is online; see [`ChannelSync::connected_member_fingerprints`].
    pub fn online_members(&self) -> Vec<String> {
        self.sync.connected_member_fingerprints()
    }

    /// What this node knows about reaching each current member, for the debug console's network
    /// view. Local state only; see [`ChannelSync::member_routes`].
    pub fn member_routes(&self) -> Vec<catcoms_sync::MemberRoute> {
        self.sync.member_routes()
    }

    /// Cheap session-local invalidation epoch for [`Self::member_routes`].
    pub fn member_route_revision(&self) -> u64 {
        self.sync.member_route_revision()
    }

    /// Session-local revision of the explicit delivery-receipt evidence. A delivery snapshot can
    /// change without any document changing (a peer's receipt arrives), so [`Self::doc_version`]
    /// alone cannot decide when to re-derive one; this is the other half.
    pub fn delivery_evidence_revision(&self) -> u64 {
        self.sync.delivery_receipt_revision()
    }

    /// Session-monotonic version of one replicated document: the number of signed ops applied to
    /// it so far (`0` for a document that is not open). Every path that changes a document's
    /// content, local edit, live gossip, past-epoch replay and catch-up alike, appends exactly
    /// one op to its log, and duplicates are dropped before they count; so an unchanged version
    /// means an unchanged projection, and a consumer can skip re-materializing it. O(1).
    pub fn doc_version(&self, doc_type: DocType, doc_id: u128) -> u64 {
        self.sync
            .doc(doc_type, doc_id)
            .map(|d| d.op_count() as u64)
            .unwrap_or(0)
    }

    /// Every inbound join attempt this node served this session, newest first, with why each was
    /// refused (the **operator's** view; see [`JoinAttempt`]).
    ///
    /// The joiner still receives an opaque rejection over the wire. This is the other half of it:
    /// without it, an expired invite, a revoked one and one that was already redeemed are the
    /// same silent failure to everyone involved.
    pub fn join_attempts(&self) -> Vec<JoinAttempt> {
        self.sync.join_attempts()
    }

    /// Milliseconds since the Unix epoch on this server's injected clock; the same seam message
    /// timestamps use, so throttles stay deterministic under a test clock.
    pub fn now_ms(&self) -> u64 {
        self.sync.now_ms()
    }

    /// Clone the injected runtime clock for actor-owned timers.
    ///
    /// Keeping throttles on this seam makes lifecycle and retry behavior deterministic in tests
    /// and avoids accidentally mixing protocol time with ambient operating-system time.
    pub(crate) fn runtime_clock(&self) -> Arc<dyn Clock + Send> {
        self.sync.runtime_clock()
    }

    /// Delivery state for this device's recent messages in `channel`, oldest first
    /// (`docs/design-delivery-states.md`, D2). Empty for a channel this session has not sent to
    ///; including every channel right after a restart, since the `message id → change` mapping
    /// is deliberately not persisted.
    ///
    /// Read-only over state that already exists: `delivered` comes from causal evidence plus
    /// already-received explicit receipts ([`ChannelSync::peers_with_change`]), `reachable` from
    /// the self-asserted route projection that drives [`Server::online_members`], and `any_peer`
    /// from a live peer that has already served a roster-verified catch-up. This query creates no
    /// wire traffic; receipt frames are sent separately when remote changes are applied.
    pub fn delivery_snapshot(&mut self, channel: u128) -> DeliverySnapshot {
        self.delivery_snapshot_revision = self.delivery_snapshot_revision.saturating_add(1);
        let revision = self.delivery_snapshot_revision;
        // Clone the bounded (<= 50) row list before querying sync, whose evidence lookup requires
        // mutable access. Holding a map borrow across that lookup would either be rejected by Rust
        // or invite a future refactor to split one coherent actor snapshot into two turns.
        let recent = self.own_message_changes.get(&channel).cloned();
        let states = recent
            .map(|recent| {
                let (ids, changes): (Vec<String>, Vec<ChangeHash>) = recent.into_iter().unzip();
                let reachable = self.online_members().len();
                let any_peer = self.sync.has_proven_connected_member_peer();
                let holders = self
                    .sync
                    .peers_with_changes(DocType::Channel, channel, &changes);
                ids.into_iter()
                    .zip(holders)
                    .map(|(id, peers)| DeliveryState {
                        id,
                        delivered: peers.len(),
                        reachable,
                        any_peer,
                    })
                    .collect()
            })
            .unwrap_or_default();
        DeliverySnapshot { revision, states }
    }

    /// Pending incoming DM (friend) requests, each as `(sender fingerprint, sender display name,
    /// opaque DM-group invite bytes)`; the name resolved against this group's profiles so the
    /// recipient sees who it's from regardless of which server is active.
    pub fn dm_requests(&self) -> Vec<(String, String, Vec<u8>)> {
        let profiles = self.profiles();
        self.sync
            .pending_dm_invites()
            .into_iter()
            .map(|(fp, invite)| {
                let name = profiles
                    .get(&fp)
                    .map(|p| p.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| fp.clone());
                (fp, name, invite)
            })
            .collect()
    }

    /// Drop a pending DM request by the sender's fingerprint (once accepted or dismissed).
    pub fn dismiss_dm_request(&mut self, from_fp: &str) {
        self.sync.dismiss_dm_invite(from_fp);
    }

    /// Deliver a DM (friend) invite to current member `target_fp` over this group ("Add friend").
    /// `Ok(true)` if delivered, `Ok(false)` if the target isn't reachable (no peer record).
    pub async fn send_dm_invite(
        &mut self,
        target_fp: &str,
        invite: &[u8],
    ) -> Result<bool, AppError> {
        Ok(self.sync.send_dm_invite(target_fp, invite).await?)
    }

    /// Push a call-signalling message (opaque payload) to a current member. `Ok(true)` if delivered,
    /// `Ok(false)` if the target isn't reachable.
    pub async fn send_call_signal(
        &mut self,
        target_fp: &str,
        payload: &[u8],
    ) -> Result<bool, AppError> {
        Ok(self.sync.send_call_signal(target_fp, payload).await?)
    }

    /// Drain inbound call-signalling messages: `(sender fingerprint, opaque payload)`.
    pub fn take_call_signals(&mut self) -> Vec<(String, Vec<u8>)> {
        self.sync.take_call_signals()
    }

    /// This call's E2E media base key (derived from the group MLS exporter) + the epoch it's keyed to.
    /// Every member derives the same key locally; it is never sent on the wire.
    pub fn media_key(&self, call_id: u128) -> Result<([u8; 32], u64), AppError> {
        Ok(self.sync.media_key(call_id)?)
    }

    /// Remove a file from the shared index; **every** listing of that content; and
    /// **garbage-collect its now-orphaned chunk blobs** from local storage. **Owner or admin
    /// only**; errors otherwise. The GC is **dedup-safe**: a chunk still referenced by another
    /// listed file (chunks are content-addressed, so two files can share one; and content dedup
    /// in [`add_file`](Self::add_file) makes several listings share *all* of them) is kept; only
    /// chunks no remaining manifest references are deleted (they're re-fetchable from any peer
    /// that still holds them). Like invites and member removal, the role gate is
    /// honest-client-enforced (the protocol residual is the same as those).
    pub async fn delete_file(&mut self, cid: &Cid) -> Result<(), AppError> {
        self.delete_file_listing(cid, None).await
    }

    /// Unlist ONE folder's listing of a file, leaving any other listing of the same content
    /// intact and downloadable. Content dedup means the same bytes can be listed under several
    /// names/folders against one set of sealed blobs (e.g. an image attached to two wiki pages);
    /// this removes just the listing in `path`, and the dedup-safe GC keeps the shared chunk
    /// blobs alive for the survivors. Same owner/admin gate as [`delete_file`](Self::delete_file),
    /// which is the "remove it everywhere" verb.
    pub async fn delete_file_at(&mut self, cid: &Cid, path: &str) -> Result<(), AppError> {
        let folder = normalize_path(path);
        self.delete_file_listing(cid, Some(&folder)).await
    }

    /// Shared body of [`delete_file`] (`folder = None`, every listing) and [`delete_file_at`]
    /// (one folder's listing): unlist, then GC the chunk blobs no *remaining* listing references.
    async fn delete_file_listing(
        &mut self,
        cid: &Cid,
        folder: Option<&str>,
    ) -> Result<(), AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) {
            return Err(AppError::Invalid(
                "only an owner or admin can delete files".into(),
            ));
        }
        let raw = cid.as_bytes().to_vec();
        let doomed = |e: &&FileEntry| {
            e.cid == raw && folder.is_none_or(|f| e.path == f) // `None` = every listing
        };
        if !self.files().iter().any(|e| doomed(&e)) {
            return Err(AppError::Invalid("no such file".into()));
        }
        // Capture the chunk blobs of the listing(s) being removed, BEFORE unlisting them.
        let removed_chunks: Vec<Cid> = self
            .files()
            .iter()
            .filter(doomed)
            .filter_map(|e| FileManifest::decode_or_legacy(&e.file_ref).ok())
            .flat_map(|m| m.chunks.into_iter().map(|c| c.ciphertext_cid))
            .collect();
        self.sync
            .post(DocType::FileIndex, FILE_INDEX_DOC, |d| {
                delete_file_entry(d, &raw, folder)
            })
            .await?;
        // Dedup-safe GC: delete each removed chunk that NO still-listed file references.
        let live: std::collections::HashSet<Cid> = self
            .files()
            .iter()
            .filter_map(|e| FileManifest::decode_or_legacy(&e.file_ref).ok())
            .flat_map(|m| m.chunks.into_iter().map(|c| c.ciphertext_cid))
            .collect();
        for chunk in removed_chunks {
            if !live.contains(&chunk) {
                let _ = self.sync.delete_blob(&chunk);
            }
        }
        Ok(())
    }

    /// Adjust ONE listing's circulation expiry: `Some(ms)` sets an absolute ms-epoch deadline,
    /// `None` means **keep forever**. Addressed by content address + folder, exactly like
    /// [`delete_file_at`](Self::delete_file_at), because expiry is per listing; the same bytes
    /// listed under two folders carry two independent deadlines.
    ///
    /// **Gate (honest-client, like the other R6 role gates):** the listing's **uploader**, the
    /// **owner**, or an **admin**. A member adjusting someone else's share is refused locally;
    /// the protocol residual is identical to invites and member removal.
    ///
    /// Setting a deadline does **not** cause anything to happen at that instant; see
    /// [`FileExpiry`]: this records metadata for a retention pass that does not exist yet, and a
    /// deadline on a wiki-embedded file is overridden by [`wiki_pinned_cids`](Self::wiki_pinned_cids)
    /// in any case.
    pub async fn set_file_expiry(
        &mut self,
        cid: &Cid,
        path: &str,
        expires: Option<u64>,
    ) -> Result<(), AppError> {
        let folder = normalize_path(path);
        let raw = cid.as_bytes().to_vec();
        let me = self.my_fingerprint();
        let listings: Vec<FileEntry> = self
            .files()
            .into_iter()
            .filter(|e| e.cid == raw && e.path == folder)
            .collect();
        if listings.is_empty() {
            return Err(AppError::Invalid("no such file".into()));
        }
        let privileged = matches!(self.my_role(), Role::Owner | Role::Admin);
        if !privileged && !listings.iter().any(|e| e.author == me) {
            return Err(AppError::Invalid(
                "only the uploader, an admin, or the owner can change a file's expiry".into(),
            ));
        }
        let expires = match expires {
            Some(ms) => FileExpiry::At(ms),
            None => FileExpiry::Never,
        };
        self.sync
            .post(DocType::FileIndex, FILE_INDEX_DOC, |d| {
                set_file_entry_expiry(d, &raw, &folder, expires)
            })
            .await?;
        Ok(())
    }

    /// Every file content address referenced by a **live wiki page body**, lowercase hex, under
    /// either marker grammar (`![alt](cid:HEX)` embeds and `[label](file:HEX)` chips).
    ///
    /// **The never-decay set.** Product rule: a file embedded in a wiki page must never drop out
    /// of circulation. Any future retention/GC pass; in this crate or in
    /// `catcoms-storage`; **MUST** consult this set and treat a member of it as un-expirable,
    /// whatever [`FileEntry::expires`] says.
    ///
    /// It is **derived, never stored**: the answer is recomputed from the wiki document every
    /// time, so editing the embed out of a page (or deleting the page) un-pins the file with no
    /// bookkeeping, and no stale pin can outlive the page that justified it. Content-addressed,
    /// so pinning one listing of some bytes pins the bytes for every listing of them.
    pub fn wiki_pinned_cids(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for body in self.wiki_map().values() {
            scan_file_markers(body, &mut out);
        }
        // Proposed bodies pin too: a file embedded in an edit awaiting review must still be
        // fetchable when the edit lands, however long the window ran.
        for p in self.wiki_pending_edits() {
            scan_file_markers(&p.body, &mut out);
        }
        out
    }

    /// Where a file is used across this server: which wiki pages embed/reference it, and how many
    /// status posts and chat messages do. Both marker grammars count.
    ///
    /// Scans the wiki map, the status document, and **every channel document open on this
    /// device** in-process (the same mechanic as [`inbox`](Self::inbox)); so the chat count is
    /// necessarily scoped to the channels this device has open and synced, not to some global
    /// truth no peer can see. Keyed by content address; see [`FileUsage`].
    pub fn file_usage(&self, cid: &Cid) -> FileUsage {
        let hex = cid.to_hex(); // lowercase, matching the scanner's normalization
        let mentions = |text: &str| {
            let mut found = HashSet::new();
            scan_file_markers(text, &mut found);
            found.contains(&hex)
        };
        let mut wiki_pages: Vec<String> = self
            .wiki_map()
            .into_iter()
            .filter(|(_, body)| mentions(body))
            .map(|(name, _)| name)
            .collect();
        wiki_pages.sort();
        let status_count = self.statuses().iter().filter(|m| mentions(&m.text)).count();
        let chat_count = self
            .sync
            .channel_ids()
            .into_iter()
            .map(|ch| {
                self.messages(ch)
                    .iter()
                    .filter(|m| mentions(&m.text))
                    .count()
            })
            .sum();
        // An event names its poster image by address directly, not through a body marker, so
        // that field is matched as well as scanned text.
        let event_count = self
            .events()
            .iter()
            .filter(|e| e.image == hex || mentions(&e.body))
            .count();
        FileUsage {
            wiki_pages,
            status_count,
            chat_count,
            event_count,
        }
    }

    /// Catch up the file index document from `peer` (e.g. right after joining).
    pub async fn request_files_catchup(&mut self, peer: PeerId) -> Result<usize, AppError> {
        Ok(self
            .sync
            .request_catchup(peer, DocType::FileIndex, FILE_INDEX_DOC)
            .await?)
    }

    /// Open (create/subscribe) the per-server **status feed**; a server-wide stream of
    /// short posts (announcements/activity), reusing the canonical message schema on its
    /// own document. Call once after founding/joining.
    pub async fn open_status(&mut self) -> Result<(), AppError> {
        self.sync.open_channel(DocType::Status, STATUS_DOC).await?;
        Ok(())
    }

    /// Post to the status feed (authored by this device's fingerprint, clock-stamped).
    ///
    /// The feed is the server addressing its members rather than a conversation between them, so
    /// a plain member is refused unless an owner/admin has opened it with
    /// [`Server::set_status_members_may_post`]. Honest-client gating, like a pin or a delete: a
    /// modified client could append the raw op regardless (the documented R6 residual), so this
    /// shapes what the app offers, not what the document can be made to hold.
    pub async fn post_status(&mut self, text: &str) -> Result<(), AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) && !self.status_members_may_post() {
            return Err(AppError::Invalid(
                "only an owner or admin can post to the status feed".into(),
            ));
        }
        let author = self.my_fingerprint();
        let ts = self.sync.now_ms();
        let id = self.sync.random_id();
        self.sync
            .post(DocType::Status, STATUS_DOC, |d| {
                append_status_message(d, &id, &author, text, ts)
            })
            .await?;
        Ok(())
    }

    /// The current status feed (oldest-first; the UI shows newest at the top).
    pub fn statuses(&self) -> Vec<ChatMessage> {
        self.sync
            .doc(DocType::Status, STATUS_DOC)
            .map(|d| read_status_messages(d.doc()))
            .unwrap_or_default()
    }

    /// Catch up the status feed from `peer` (e.g. right after joining).
    pub async fn request_status_catchup(&mut self, peer: PeerId) -> Result<usize, AppError> {
        Ok(self
            .sync
            .request_catchup(peer, DocType::Status, STATUS_DOC)
            .await?)
    }

    /// Edit the text of one of **your own** status posts (by id). Authorship is the whole claim to
    /// a post's wording, so not even an owner may reword someone else's; moderation removes a post
    /// (`delete_status`) rather than putting different words in its author's mouth. Honest-client
    /// gating, as everywhere in a CRDT (see THREAT-MODEL.md). A no-op edit (same text) is dropped,
    /// so the `post` always carries a real change.
    ///
    /// An empty `id` is refused first, exactly as in [`Server::edit_message`]: it is the id every
    /// post written before ids existed carries, so the author check below would pass on the
    /// caller's own id-less post while the write landed on the first id-less post in the document,
    /// whoever wrote it. Rewording somebody else's announcement is the one thing this method says
    /// cannot happen, so the guard sits here and not in the composer: the command is reachable from
    /// the webview directly, and honest-client gating only holds while the id names one post.
    pub async fn edit_status(&mut self, id: &str, new_text: &str) -> Result<(), AppError> {
        if !valid_status_post_id(id) {
            return Err(AppError::Invalid(
                "a status post with a malformed id cannot be addressed".into(),
            ));
        }
        let me = self.my_fingerprint();
        let Some(current) = self
            .statuses()
            .into_iter()
            .find(|m| m.id == id && m.author == me)
        else {
            return Err(AppError::Invalid(
                "you can only edit your own status posts".into(),
            ));
        };
        if current.text == new_text {
            return Ok(()); // unchanged; don't post a redundant op
        }
        let edited = self.sync.now_ms();
        let id = id.to_string();
        let new_text = new_text.to_string();
        self.sync
            .post(DocType::Status, STATUS_DOC, move |d| {
                edit_status_message_in_doc(d, &id, &new_text, edited).map(|_| ())
            })
            .await?;
        Ok(())
    }

    /// Delete a status post (by id): **your own**, or; if you are the owner/admin; anyone's
    /// (moderation). Honest-client gating, exactly as for a chat message (the documented R6
    /// residual). Errors if the post is gone or you may not delete it.
    ///
    /// An empty `id` is refused for the reason [`Server::edit_status`] gives: it names every post
    /// written before ids existed rather than one of them, so the delete would land on whichever of
    /// those the document happens to hold first.
    pub async fn delete_status(&mut self, id: &str) -> Result<(), AppError> {
        if !valid_status_post_id(id) {
            return Err(AppError::Invalid(
                "a status post with a malformed id cannot be addressed".into(),
            ));
        }
        let me = self.my_fingerprint();
        let Some(post) = self.statuses().into_iter().find(|m| m.id == id) else {
            return Err(AppError::Invalid("no such status post".into()));
        };
        let moderator = matches!(self.my_role(), Role::Owner | Role::Admin);
        if post.author != me && !moderator {
            return Err(AppError::Invalid(
                "you can only delete your own status posts".into(),
            ));
        }
        let id = id.to_string();
        self.sync
            .post(DocType::Status, STATUS_DOC, move |d| {
                delete_status_message_in_doc(d, &id).map(|_| ())
            })
            .await?;
        Ok(())
    }

    /// Toggle this member's `emoji` reaction on the status post `id` (add if absent, remove if
    /// present). **Anyone may react**, whoever may post: reading the feed is the one thing every
    /// member does, and a reaction is how they answer it. Errors if the post doesn't exist.
    ///
    /// An empty `id` is refused with the other trust-boundary checks, for the reason
    /// [`Server::edit_status`] gives: it names every post written before ids existed rather than
    /// one of them, so the reaction would land on whichever the document holds first.
    pub async fn toggle_status_reaction(&mut self, id: &str, emoji: &str) -> Result<(), AppError> {
        // Enforce the flat-key invariant at the trust boundary, before anything reaches the doc.
        if !valid_reaction_emoji(emoji) {
            return Err(AppError::Invalid("bad emoji".into()));
        }
        if !valid_status_post_id(id) {
            return Err(AppError::Invalid(
                "a status post with a malformed id cannot be addressed".into(),
            ));
        }
        let me = self.my_fingerprint();
        if !self.statuses().iter().any(|m| m.id == id) {
            return Err(AppError::Invalid("no such status post".into()));
        }
        let id = id.to_string();
        let emoji = emoji.to_string();
        self.sync
            .post(DocType::Status, STATUS_DOC, move |d| {
                toggle_status_reaction_in_doc(d, &id, &emoji, &me).map(|_| ())
            })
            .await?;
        Ok(())
    }

    /// Pin or unpin a status post (by id): the feed's way of holding an announcement above the
    /// scroll. **Owner/admin only** (honest-client gating, like a channel pin). Several posts may
    /// be pinned at once, as in a channel. Errors if the post is gone, you may not pin, or the pin
    /// state is already as requested (no redundant op).
    ///
    /// An empty `id` is refused for the reason [`Server::edit_status`] gives: it names every post
    /// written before ids existed rather than one of them, so the pin would land on whichever the
    /// document holds first.
    pub async fn set_status_pin(&mut self, id: &str, pinned: bool) -> Result<(), AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) {
            return Err(AppError::Invalid(
                "only an owner/admin can pin status posts".into(),
            ));
        }
        if !valid_status_post_id(id) {
            return Err(AppError::Invalid(
                "a status post with a malformed id cannot be addressed".into(),
            ));
        }
        let Some(post) = self.statuses().into_iter().find(|m| m.id == id) else {
            return Err(AppError::Invalid("no such status post".into()));
        };
        if post.pinned == pinned {
            return Ok(()); // already in the requested state; don't post a redundant op
        }
        let id = id.to_string();
        self.sync
            .post(DocType::Status, STATUS_DOC, move |d| {
                set_status_pin_in_doc(d, &id, pinned).map(|_| ())
            })
            .await?;
        Ok(())
    }

    /// Whether plain members may post to the status feed. **`false` by default**, so a server that
    /// has never touched the setting; including every server founded before it existed; keeps the
    /// feed as an announcement channel until someone deliberately opens it.
    pub fn status_members_may_post(&self) -> bool {
        self.sync
            .doc(DocType::Status, STATUS_DOC)
            .map(|d| read_status_may_post(d.doc()))
            .unwrap_or_default()
    }

    /// Open (`true`) or close (`false`) the status feed to plain members. **Owner or admin only**
    /// (honest-client gating, like the wiki review window). An unchanged policy is a no-op, so
    /// re-asserting the current setting costs no op.
    pub async fn set_status_members_may_post(&mut self, allow: bool) -> Result<(), AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) {
            return Err(AppError::Invalid(
                "only an owner or admin can change who may post to the status feed".into(),
            ));
        }
        if self.status_members_may_post() == allow {
            return Ok(()); // already the requested policy; don't post a redundant op
        }
        self.sync
            .post(DocType::Status, STATUS_DOC, move |d| {
                write_status_may_post(d, allow)
            })
            .await?;
        Ok(())
    }

    /// Open (create/subscribe) the per-server **calendar**; the shared document holding the
    /// server's scheduled events. Call once after founding/joining.
    pub async fn open_calendar(&mut self) -> Result<(), AppError> {
        self.sync
            .open_channel(DocType::Calendar, CALENDAR_DOC)
            .await?;
        Ok(())
    }

    /// Create a server event (authored by this device's fingerprint, clock-stamped); replies with
    /// its fresh id. **Any member may**; an event is server *content*, like a channel, a status
    /// post or a channel topic, so this is deliberately not owner/admin gated. The op is
    /// inner-signed by this device, so authorship is attributable exactly as for a message.
    ///
    /// `image` is the hex content address of an already-shared file (empty for none); it is
    /// checked for shape only, not for presence, since the blob may still be in flight to this
    /// device when the event is written.
    ///
    /// Rejects a blank title, a title over [`MAX_EVENT_TITLE_BYTES`] / a body over
    /// [`MAX_EVENT_BODY_BYTES`] UTF-8 bytes, an `end_ts` before `start_ts` (`0` = no end), and an
    /// `image` that is not a hex address of at most [`MAX_EVENT_IMAGE_HEX`] digits.
    pub async fn create_event(
        &mut self,
        title: &str,
        body: &str,
        start_ts: u64,
        end_ts: u64,
        image: &str,
    ) -> Result<String, AppError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::Invalid("an event needs a title".into()));
        }
        if title.len() > MAX_EVENT_TITLE_BYTES {
            return Err(AppError::Invalid(format!(
                "event title too long: {} bytes (max {MAX_EVENT_TITLE_BYTES})",
                title.len()
            )));
        }
        if body.len() > MAX_EVENT_BODY_BYTES {
            return Err(AppError::Invalid(format!(
                "event body too long: {} bytes (max {MAX_EVENT_BODY_BYTES})",
                body.len()
            )));
        }
        if end_ts != 0 && end_ts < start_ts {
            return Err(AppError::Invalid(
                "an event cannot end before it starts".into(),
            ));
        }
        let image = image.trim().to_ascii_lowercase();
        if !image.is_empty()
            && (image.len() > MAX_EVENT_IMAGE_HEX || !image.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(AppError::Invalid(
                "an event image must be a file content address".into(),
            ));
        }
        let event = ServerEvent {
            id: self.sync.random_id(),
            title: title.to_string(),
            body: body.to_string(),
            start_ts,
            end_ts,
            author: self.my_fingerprint(),
            created_ts: self.sync.now_ms(),
            image,
        };
        let id = event.id.clone();
        self.sync
            .post(DocType::Calendar, CALENDAR_DOC, |d| write_event(d, &event))
            .await?;
        Ok(id)
    }

    /// Delete a server event (by id): **your own**, or; if you are the owner/admin; anyone's
    /// (moderation), exactly like [`Server::delete_message`]. Honest-client gating (a modified
    /// client could post a raw delete op regardless; the documented R6 residual). Errors if the
    /// event is gone or you may not delete it.
    pub async fn delete_event(&mut self, id: &str) -> Result<(), AppError> {
        let me = self.my_fingerprint();
        let Some(event) = self.events().into_iter().find(|e| e.id == id) else {
            return Err(AppError::Invalid("no such event".into()));
        };
        let moderator = matches!(self.my_role(), Role::Owner | Role::Admin);
        if event.author != me && !moderator {
            return Err(AppError::Invalid(
                "you can only delete your own events".into(),
            ));
        }
        let id = id.to_string();
        self.sync
            .post(DocType::Calendar, CALENDAR_DOC, move |d| {
                delete_event_in_doc(d, &id).map(|_| ())
            })
            .await?;
        Ok(())
    }

    /// Every server event, sorted by **start time ascending** (empty if none, or if the calendar
    /// is not open).
    pub fn events(&self) -> Vec<ServerEvent> {
        self.sync
            .doc(DocType::Calendar, CALENDAR_DOC)
            .map(|d| read_events(d.doc()))
            .unwrap_or_default()
    }

    /// Catch up the calendar document from `peer` (e.g. right after joining).
    pub async fn request_calendar_catchup(&mut self, peer: PeerId) -> Result<usize, AppError> {
        Ok(self
            .sync
            .request_catchup(peer, DocType::Calendar, CALENDAR_DOC)
            .await?)
    }

    /// Open (create/subscribe) the per-server **wiki** document. Call once after
    /// founding/joining.
    pub async fn open_wiki(&mut self) -> Result<(), AppError> {
        self.sync.open_channel(DocType::Wiki, WIKI_DOC).await?;
        Ok(())
    }

    /// The full wiki as a `page name -> body` map: the **effective** wiki, with pending edits
    /// past their review deadline overlaid as accepted (read-time auto-acceptance; see the
    /// `\u{0}hist` module comment). Every consumer (pages, bodies, backlinks, pinning) reads
    /// through this, so an auto-accepted edit is live everywhere at once.
    pub fn wiki_map(&self) -> HashMap<String, String> {
        let now = self.sync.now_ms();
        self.sync
            .doc(DocType::Wiki, WIKI_DOC)
            .map(|d| {
                let doc = d.doc();
                let mut map = read_wiki_map(doc);
                overlay_accepted_pending(doc, now, &mut map);
                map
            })
            .unwrap_or_default()
    }

    /// The wiki's page names, sorted.
    pub fn wiki_pages(&self) -> Vec<String> {
        let mut pages: Vec<String> = self.wiki_map().into_keys().collect();
        pages.sort();
        pages
    }

    /// A wiki page's body (empty if the page does not exist).
    pub fn read_wiki_page(&self, name: &str) -> String {
        self.wiki_map().get(name).cloned().unwrap_or_default()
    }

    /// The wiki's per-page render formats (`page name -> "md" | "wiki"`). A page missing from
    /// this map renders as markdown (the default).
    pub fn wiki_meta(&self) -> HashMap<String, String> {
        self.sync
            .doc(DocType::Wiki, WIKI_DOC)
            .map(|d| read_wiki_meta(d.doc()))
            .unwrap_or_default()
    }

    /// Create or update a wiki page (the body merges character-by-character).
    ///
    /// Returns whether the edit was **queued for review** rather than published: with the
    /// server's review mode on (`wiki_review_days() > 0`), a plain member's save becomes a
    /// [`WikiPendingEdit`] that an owner/admin approves or declines, auto-accepting at its
    /// deadline; owner/admin saves always publish immediately. `Ok(false)` = live now.
    pub async fn write_wiki_page(&mut self, name: &str, body: &str) -> Result<bool, AppError> {
        self.write_wiki_page_as(name, body, "edit", "").await
    }

    /// The shared write path behind [`write_wiki_page`](Self::write_wiki_page) and
    /// [`restore_wiki_page`](Self::restore_wiki_page): review-gate, fold, write, record.
    async fn write_wiki_page_as(
        &mut self,
        name: &str,
        body: &str,
        kind: &str,
        note: &str,
    ) -> Result<bool, AppError> {
        let name = valid_wiki_name(name)?;
        let me = self.my_fingerprint();
        let now = self.sync.now_ms();
        let review = self.wiki_review_days();
        if review > 0 && matches!(self.my_role(), Role::Member) {
            let pending = WikiPendingEdit {
                id: self.sync.random_id(),
                page: name,
                author: me,
                ts: now,
                expires_ts: now.saturating_add(u64::from(review) * DAY_MS),
                body: body.to_string(),
            };
            self.sync
                .post(DocType::Wiki, WIKI_DOC, |d| write_wiki_pending(d, &pending))
                .await?;
            return Ok(true);
        }
        let rev = WikiRevision {
            id: self.sync.random_id(),
            author: me,
            ts: now,
            body: body.to_string(),
            kind: kind.to_string(),
            actor: String::new(),
            note: note.to_string(),
        };
        self.sync
            .post(DocType::Wiki, WIKI_DOC, |d| {
                fold_expired_wiki_pending(d, &name, now)?;
                write_wiki_page(d, &name, body)?;
                append_wiki_rev(d, &name, &rev)
            })
            .await?;
        Ok(false)
    }

    /// A page's revision history, oldest first: the stored revisions plus a synthesized
    /// `"auto"` entry for every pending edit past its deadline that no write has folded in
    /// yet (so the reader always sees acceptance the moment it happens, storage or not).
    pub fn wiki_history(&self, page: &str) -> Vec<WikiRevision> {
        let now = self.sync.now_ms();
        self.sync
            .doc(DocType::Wiki, WIKI_DOC)
            .map(|d| {
                let doc = d.doc();
                let mut revs = read_wiki_history(doc, page);
                for p in expired_wiki_pending(doc, now) {
                    if p.page == page && !revs.iter().any(|r| r.id == p.id) {
                        revs.push(WikiRevision {
                            id: p.id,
                            author: p.author,
                            ts: p.expires_ts,
                            body: p.body,
                            kind: "auto".into(),
                            actor: String::new(),
                            note: String::new(),
                        });
                    }
                }
                revs.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.id.cmp(&b.id)));
                revs
            })
            .unwrap_or_default()
    }

    /// The live review queue: pending edits still inside their window, oldest first. An edit
    /// past its deadline is auto-accepted and no longer reviewable, so it is not listed.
    pub fn wiki_pending_edits(&self) -> Vec<WikiPendingEdit> {
        let now = self.sync.now_ms();
        self.sync
            .doc(DocType::Wiki, WIKI_DOC)
            .map(|d| {
                read_wiki_pending_all(d.doc())
                    .into_iter()
                    .filter(|p| p.expires_ts > now)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The server's wiki review window in days; `0` = edits publish immediately (the default).
    pub fn wiki_review_days(&self) -> u32 {
        self.sync
            .doc(DocType::Wiki, WIKI_DOC)
            .map(|d| read_wiki_review_days(d.doc()))
            .unwrap_or_default()
    }

    /// Set the wiki review window: member edits wait up to `days` (max
    /// [`MAX_WIKI_REVIEW_DAYS`]) for owner/admin review, then auto-accept; `0` turns review
    /// off. **Owner or admin only** (honest-client gating, like the livery).
    pub async fn set_wiki_review_days(&mut self, days: u32) -> Result<(), AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) {
            return Err(AppError::Invalid(
                "only an owner or admin can set the wiki review window".into(),
            ));
        }
        if days > MAX_WIKI_REVIEW_DAYS {
            return Err(AppError::Invalid(format!(
                "review window longer than {MAX_WIKI_REVIEW_DAYS} days"
            )));
        }
        self.sync
            .post(DocType::Wiki, WIKI_DOC, |d| write_wiki_review_days(d, days))
            .await?;
        Ok(())
    }

    /// Approve a pending edit: publish its body and record an `"approve"` revision (id = the
    /// pending id, author = the proposer, actor = the approver). **Owner or admin only.**
    pub async fn approve_wiki_edit(&mut self, pending_id: &str) -> Result<(), AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) {
            return Err(AppError::Invalid(
                "only an owner or admin can approve a wiki edit".into(),
            ));
        }
        let now = self.sync.now_ms();
        let Some(pending) = self
            .sync
            .doc(DocType::Wiki, WIKI_DOC)
            .map(|d| read_wiki_pending_all(d.doc()))
            .unwrap_or_default()
            .into_iter()
            .find(|p| p.id == pending_id)
        else {
            return Err(AppError::Invalid(format!(
                "no pending wiki edit {pending_id:?}"
            )));
        };
        let rev = WikiRevision {
            id: pending.id.clone(),
            author: pending.author.clone(),
            ts: now,
            body: pending.body.clone(),
            kind: "approve".into(),
            actor: self.my_fingerprint(),
            note: String::new(),
        };
        self.sync
            .post(DocType::Wiki, WIKI_DOC, |d| {
                // Fold first: an OLDER proposal for this page that lapsed while this one waited
                // is already the live body by the read-time overlay. Left in the queue it would
                // keep overriding the body this approval just published, so it becomes stored
                // history here and stops being an overlay.
                fold_expired_wiki_pending(d, &pending.page, now)?;
                write_wiki_page(d, &pending.page, &pending.body)?;
                append_wiki_rev(d, &pending.page, &rev)?;
                delete_wiki_pending(d, &pending.id)
            })
            .await?;
        Ok(())
    }

    /// Decline a pending edit: drop it from the queue and record a `"reject"` revision holding
    /// the proposed body (auditable, never live). **Owner or admin only.** An edit past its
    /// deadline has already auto-accepted and errors; roll the page back instead.
    pub async fn reject_wiki_edit(&mut self, pending_id: &str) -> Result<(), AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) {
            return Err(AppError::Invalid(
                "only an owner or admin can decline a wiki edit".into(),
            ));
        }
        let now = self.sync.now_ms();
        let Some(pending) = self
            .sync
            .doc(DocType::Wiki, WIKI_DOC)
            .map(|d| read_wiki_pending_all(d.doc()))
            .unwrap_or_default()
            .into_iter()
            .find(|p| p.id == pending_id)
        else {
            return Err(AppError::Invalid(format!(
                "no pending wiki edit {pending_id:?}"
            )));
        };
        if pending.expires_ts <= now {
            return Err(AppError::Invalid(
                "this edit already auto-accepted; roll the page back instead".into(),
            ));
        }
        let rev = WikiRevision {
            id: pending.id.clone(),
            author: pending.author.clone(),
            ts: now,
            body: pending.body.clone(),
            kind: "reject".into(),
            actor: self.my_fingerprint(),
            note: String::new(),
        };
        self.sync
            .post(DocType::Wiki, WIKI_DOC, |d| {
                // Same fold as on approval: a lapsed older proposal is settled content, not a
                // standing overlay, and declining this one must not leave that ambiguous.
                fold_expired_wiki_pending(d, &pending.page, now)?;
                append_wiki_rev(d, &pending.page, &rev)?;
                delete_wiki_pending(d, &pending.id)
            })
            .await?;
        Ok(())
    }

    /// Restore a page to an earlier revision's body. Routed through the normal write path, so
    /// with review mode on a plain member's restore queues like any other edit (returns
    /// `true`); otherwise it publishes a `"rollback"` revision noting the restored id.
    pub async fn restore_wiki_page(&mut self, page: &str, rev_id: &str) -> Result<bool, AppError> {
        let page = valid_wiki_name(page)?;
        let Some(rev) = self
            .wiki_history(&page)
            .into_iter()
            .find(|r| r.id == rev_id)
        else {
            return Err(AppError::Invalid(format!(
                "no revision {rev_id:?} of wiki page {page:?}"
            )));
        };
        self.write_wiki_page_as(&page, &rev.body, "rollback", rev_id)
            .await
    }

    /// Set a wiki page's render format; `"md"` or `"wiki"`; any other value is refused.
    ///
    /// The format lives in a reserved NUL-prefixed root key holding a `Map`, which older peers'
    /// readers skip, so declaring a format never disturbs a peer that predates the feature.
    pub async fn set_wiki_page_format(&mut self, name: &str, format: &str) -> Result<(), AppError> {
        let name = valid_wiki_name(name)?;
        if !WIKI_FORMATS.contains(&format) {
            return Err(AppError::Invalid(format!(
                "unknown wiki format {format:?} (expected \"md\" or \"wiki\")"
            )));
        }
        let format = format.to_string();
        self.sync
            .post(DocType::Wiki, WIKI_DOC, |d| {
                set_wiki_format(d, &name, &format)
            })
            .await?;
        Ok(())
    }

    /// Delete a wiki page (and its format metadata). Errors if the page does not exist. With
    /// review mode on, **owner/admin only** (a delete would sidestep review); its open
    /// proposals are dropped with it, and a `"delete"` revision preserves the last body.
    pub async fn delete_wiki_page(&mut self, name: &str) -> Result<(), AppError> {
        let name = valid_wiki_name(name)?;
        if self.wiki_review_days() > 0 && matches!(self.my_role(), Role::Member) {
            return Err(AppError::Invalid(
                "only an owner or admin can delete a page while edit review is on".into(),
            ));
        }
        let Some(last_body) = self.wiki_map().get(&name).cloned() else {
            return Err(AppError::Invalid(format!("no such wiki page {name:?}")));
        };
        let now = self.sync.now_ms();
        let rev = WikiRevision {
            id: self.sync.random_id(),
            author: self.my_fingerprint(),
            ts: now,
            body: last_body,
            kind: "delete".into(),
            actor: String::new(),
            note: String::new(),
        };
        self.sync
            .post(DocType::Wiki, WIKI_DOC, |d| {
                fold_expired_wiki_pending(d, &name, now)?;
                let open: Vec<String> = read_wiki_pending_all(d)
                    .into_iter()
                    .filter(|p| p.page == name)
                    .map(|p| p.id)
                    .collect();
                for id in open {
                    delete_wiki_pending(d, &id)?;
                }
                append_wiki_rev(d, &name, &rev)?;
                delete_wiki_page_op(d, &name)
            })
            .await?;
        Ok(())
    }

    /// Rename a wiki page, carrying its body and format. Errors if `from` does not exist or
    /// `to` already does (a rename never silently clobbers a page). With review mode on,
    /// **owner/admin only**; open proposals follow the page to its new name, and a
    /// `"rename"` revision on the new name notes where it came from.
    pub async fn rename_wiki_page(&mut self, from: &str, to: &str) -> Result<(), AppError> {
        let from = valid_wiki_name(from)?;
        let to = valid_wiki_name(to)?;
        if self.wiki_review_days() > 0 && matches!(self.my_role(), Role::Member) {
            return Err(AppError::Invalid(
                "only an owner or admin can rename a page while edit review is on".into(),
            ));
        }
        let pages = self.wiki_map();
        let Some(body) = pages.get(&from).cloned() else {
            return Err(AppError::Invalid(format!("no such wiki page {from:?}")));
        };
        if from != to && pages.contains_key(&to) {
            return Err(AppError::Invalid(format!(
                "wiki page {to:?} already exists"
            )));
        }
        if from == to {
            return Ok(());
        }
        let now = self.sync.now_ms();
        let rev = WikiRevision {
            id: self.sync.random_id(),
            author: self.my_fingerprint(),
            ts: now,
            body: body.clone(),
            kind: "rename".into(),
            actor: String::new(),
            note: from.clone(),
        };
        self.sync
            .post(DocType::Wiki, WIKI_DOC, |d| {
                fold_expired_wiki_pending(d, &from, now)?;
                rename_wiki_page_op(d, &from, &to)?;
                // The stored body the op copied can trail the effective one (an accepted
                // pending not folded elsewhere); write the body every reader saw.
                write_wiki_page(d, &to, &body)?;
                repoint_wiki_pending(d, &from, &to)?;
                append_wiki_rev(d, &to, &rev)
            })
            .await?;
        Ok(())
    }

    /// Catch up the wiki from `peer` (e.g. right after joining).
    pub async fn request_wiki_catchup(&mut self, peer: PeerId) -> Result<usize, AppError> {
        Ok(self
            .sync
            .request_catchup(peer, DocType::Wiki, WIKI_DOC)
            .await?)
    }

    /// Catch up the profile document from `peer` (e.g. right after joining).
    pub async fn request_profiles_catchup(&mut self, peer: PeerId) -> Result<usize, AppError> {
        Ok(self
            .sync
            .request_catchup(peer, DocType::Profile, PROFILE_DOC)
            .await?)
    }

    /// Open (create/subscribe) the per-server **livery** document. Call once after
    /// founding/joining.
    pub async fn open_livery(&mut self) -> Result<(), AppError> {
        self.sync.open_channel(DocType::Livery, LIVERY_DOC).await?;
        Ok(())
    }

    /// Publish the server livery. **Owner or admin only**; errors otherwise (honest-client
    /// gating, the same policy layer as roles/pins). An all-empty [`Livery`] removes it.
    /// Values are opaque here and bounded only by size (the client validates them on read);
    /// an over-long field or too many tokens is rejected, like an over-large avatar.
    ///
    /// `livery.icon` and `livery.cursor` are **ignored**: this is a read-modify-write of
    /// preset/accent/tokens that carries the stored images through unchanged, so republishing
    /// colours never resends them (nor clears them). Use [`Server::set_server_icon`] /
    /// [`Server::set_server_cursor`] to change those.
    pub async fn set_livery(&mut self, livery: Livery) -> Result<(), AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) {
            return Err(AppError::Invalid(
                "only an owner or admin can set the livery".into(),
            ));
        }
        if livery.preset.len() > MAX_LIVERY_PRESET_BYTES {
            return Err(AppError::Invalid(format!(
                "livery preset too long: {} bytes (max {MAX_LIVERY_PRESET_BYTES})",
                livery.preset.len()
            )));
        }
        if livery.accent.len() > MAX_LIVERY_ACCENT_BYTES {
            return Err(AppError::Invalid(format!(
                "livery accent too long: {} bytes (max {MAX_LIVERY_ACCENT_BYTES})",
                livery.accent.len()
            )));
        }
        if livery.tokens.len() > MAX_LIVERY_TOKENS {
            return Err(AppError::Invalid(format!(
                "too many livery tokens: {} (max {MAX_LIVERY_TOKENS})",
                livery.tokens.len()
            )));
        }
        for (k, v) in &livery.tokens {
            if k.len() > MAX_LIVERY_TOKEN_KEY_BYTES {
                return Err(AppError::Invalid(format!(
                    "livery token name too long: {} bytes (max {MAX_LIVERY_TOKEN_KEY_BYTES})",
                    k.len()
                )));
            }
            if v.len() > MAX_LIVERY_TOKEN_VALUE_BYTES {
                return Err(AppError::Invalid(format!(
                    "livery token value too long: {} bytes (max {MAX_LIVERY_TOKEN_VALUE_BYTES})",
                    v.len()
                )));
            }
        }
        self.sync
            .post(DocType::Livery, LIVERY_DOC, |d| {
                // Read-modify-write inside the edit itself: take the images that are already in
                // the document and write them back verbatim, so a colour publish is a no-op for
                // the (comparatively huge) icon and the cursor alike.
                let kept = Livery {
                    icon: str_field(d, &ROOT, L_ICON),
                    cursor: str_field(d, &ROOT, L_CURSOR),
                    ..livery
                };
                write_livery(d, &kept)
            })
            .await?;
        Ok(())
    }

    /// Set (or clear, with `""`) the shared **server icon**; base64 image bytes stored in
    /// the livery document. **Owner or admin only**, exactly like [`Server::set_livery`],
    /// which leaves this value alone. Rejects malformed base64 and anything over
    /// [`MAX_SERVER_ICON_BYTES`] decoded bytes, the same way an over-large avatar is rejected.
    pub async fn set_server_icon(&mut self, icon: String) -> Result<(), AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) {
            return Err(AppError::Invalid(
                "only an owner or admin can set the server icon".into(),
            ));
        }
        if !icon.is_empty() {
            let bytes = B64
                .decode(icon.as_bytes())
                .map_err(|e| AppError::Invalid(format!("bad server icon: {e}")))?;
            if bytes.len() > MAX_SERVER_ICON_BYTES {
                return Err(AppError::Invalid(format!(
                    "server icon too large: {} bytes (max {MAX_SERVER_ICON_BYTES})",
                    bytes.len()
                )));
            }
        }
        self.sync
            .post(DocType::Livery, LIVERY_DOC, |d| write_server_icon(d, &icon))
            .await?;
        Ok(())
    }

    /// Set (or clear, with `""`) the shared **server cursor**; base64 image bytes stored in
    /// the livery document, the exact mirror of [`Server::set_server_icon`]. **Owner or admin
    /// only**, and likewise left alone by [`Server::set_livery`] and by an icon write. Rejects
    /// malformed base64 and anything over [`MAX_SERVER_CURSOR_BYTES`] decoded bytes.
    pub async fn set_server_cursor(&mut self, cursor: String) -> Result<(), AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) {
            return Err(AppError::Invalid(
                "only an owner or admin can set the server cursor".into(),
            ));
        }
        if !cursor.is_empty() {
            let bytes = B64
                .decode(cursor.as_bytes())
                .map_err(|e| AppError::Invalid(format!("bad server cursor: {e}")))?;
            if bytes.len() > MAX_SERVER_CURSOR_BYTES {
                return Err(AppError::Invalid(format!(
                    "server cursor too large: {} bytes (max {MAX_SERVER_CURSOR_BYTES})",
                    bytes.len()
                )));
            }
        }
        self.sync
            .post(DocType::Livery, LIVERY_DOC, |d| {
                write_server_cursor(d, &cursor)
            })
            .await?;
        Ok(())
    }

    /// The server's published livery (all-empty if none was published).
    pub fn livery(&self) -> Livery {
        self.sync
            .doc(DocType::Livery, LIVERY_DOC)
            .map(|d| read_livery(d.doc()))
            .unwrap_or_default()
    }

    /// Catch up the livery document from `peer` (e.g. right after joining).
    pub async fn request_livery_catchup(&mut self, peer: PeerId) -> Result<usize, AppError> {
        Ok(self
            .sync
            .request_catchup(peer, DocType::Livery, LIVERY_DOC)
            .await?)
    }

    // --- custom member badges ----------------------------------------------

    /// Open (create/subscribe) the per-server **badge** document. Call once after
    /// founding/joining.
    pub async fn open_badges(&mut self) -> Result<(), AppError> {
        self.sync.open_channel(DocType::Badges, BADGES_DOC).await?;
        Ok(())
    }

    /// Assign a custom badge to the member with device fingerprint `fp`. **Owner or admin
    /// only**; errors otherwise (honest-client gating, the same policy layer as the livery).
    /// An empty `label` (or one that is only whitespace) **removes** that member's badge.
    ///
    /// The label/colour are opaque here and bounded only by size (the client validates the
    /// colour on read), except that a label reserved for a built-in role is rejected; a
    /// custom badge must never be able to read as `ADMIN`.
    pub async fn set_member_badge(
        &mut self,
        fp: String,
        label: String,
        color: String,
    ) -> Result<(), AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) {
            return Err(AppError::Invalid(
                "only an owner or admin can set a member badge".into(),
            ));
        }
        if fp.len() > MAX_BADGE_FINGERPRINT_BYTES {
            return Err(AppError::Invalid(format!(
                "badge fingerprint too long: {} bytes (max {MAX_BADGE_FINGERPRINT_BYTES})",
                fp.len()
            )));
        }
        if label.len() > MAX_BADGE_LABEL_BYTES {
            return Err(AppError::Invalid(format!(
                "badge label too long: {} bytes (max {MAX_BADGE_LABEL_BYTES})",
                label.len()
            )));
        }
        if color.len() > MAX_BADGE_COLOR_BYTES {
            return Err(AppError::Invalid(format!(
                "badge color too long: {} bytes (max {MAX_BADGE_COLOR_BYTES})",
                color.len()
            )));
        }
        if is_reserved_badge_label(&label) {
            return Err(AppError::Invalid(format!(
                "badge label '{}' is reserved for roles",
                label.trim()
            )));
        }
        // Removal is never capped; only a *new* entry can push the document over the limit.
        let label = if label.trim().is_empty() {
            String::new()
        } else {
            label
        };
        if !label.is_empty() {
            let current = self.badges();
            if !current.contains_key(&fp) && current.len() >= MAX_BADGES {
                return Err(AppError::Invalid(format!(
                    "too many member badges: {} (max {MAX_BADGES})",
                    current.len()
                )));
            }
        }
        let badge = MemberBadge { label, color };
        self.sync
            .post(DocType::Badges, BADGES_DOC, |d| {
                write_member_badge(d, &fp, &badge)
            })
            .await?;
        Ok(())
    }

    /// Every assigned member badge, keyed by device fingerprint (empty if none were assigned).
    pub fn badges(&self) -> HashMap<String, MemberBadge> {
        self.sync
            .doc(DocType::Badges, BADGES_DOC)
            .map(|d| read_badges(d.doc()))
            .unwrap_or_default()
    }

    /// Catch up the badge document from `peer` (e.g. right after joining).
    pub async fn request_badges_catchup(&mut self, peer: PeerId) -> Result<usize, AppError> {
        Ok(self
            .sync
            .request_catchup(peer, DocType::Badges, BADGES_DOC)
            .await?)
    }

    // --- companion devices (multi-device M3) -------------------------------

    /// Open (create/subscribe) the per-server **companion-device registry**. Call once after
    /// founding/joining.
    pub async fn open_devices(&mut self) -> Result<(), AppError> {
        self.sync
            .open_channel(DocType::Devices, DEVICES_DOC)
            .await?;
        Ok(())
    }

    /// Every admitted companion device, keyed by its own fingerprint. Empty until some member
    /// pairs a second device. Only entries whose stored certificate genuinely verifies for this
    /// group are returned; see the module notes on the `Devices` document.
    pub fn devices(&self) -> HashMap<String, DeviceEntry> {
        self.device_certs()
            .into_iter()
            .map(|c| {
                (
                    fingerprint(&c.new_device_id),
                    DeviceEntry {
                        origin: fingerprint(&c.origin_id),
                        name: c.device_name,
                    },
                )
            })
            .collect()
    }

    /// The **member identity** an author fingerprint belongs to: the certifying origin's
    /// fingerprint if `fp` is a companion device, otherwise `fp` itself.
    ///
    /// This is the whole point of the registry; profiles, roles and badges stay keyed by the
    /// origin, so a companion's message renders under the member's name (M4 adds the device tag
    /// beside it) with no document re-keying anywhere.
    pub fn origin_of(&self, fp: &str) -> String {
        self.devices()
            .get(fp)
            .map(|d| d.origin.clone())
            .unwrap_or_else(|| fp.to_string())
    }

    /// Catch up the device registry from `peer` (e.g. right after joining).
    pub async fn request_devices_catchup(&mut self, peer: PeerId) -> Result<usize, AppError> {
        let applied = self
            .sync
            .request_catchup(peer, DocType::Devices, DEVICES_DOC)
            .await?;
        Ok(applied)
    }

    /// The **designated committer's** (server owner's) signature public key.
    ///
    /// Read by the grant ceremony: a companion device has no roster before it is admitted, so the
    /// origin captures this key from its *live* group and puts it in the grant, and the companion
    /// pins it to authenticate the owner's Welcome; the role `InviteToken::inviter_public_key`
    /// plays for an invited joiner. `None` only for a group with no members (impossible in
    /// practice) or a non-Ed25519 roster key.
    pub fn owner_public_key(&self) -> Option<[u8; 32]> {
        let owner = self.sync.designated_committer_id()?;
        self.sync.member_public_key(&owner)?.try_into().ok()
    }

    /// The validated certificates in the registry (empty if the document is not open, or the
    /// group somehow has no owner).
    fn device_certs(&self) -> Vec<DeviceCertificate> {
        let group_id = self.sync.group_id();
        let Some(owner) = self.sync.designated_committer_id() else {
            return Vec::new();
        };
        self.sync
            .doc(DocType::Devices, DEVICES_DOC)
            .map(|d| read_device_certs(d.doc(), &group_id, &owner))
            .unwrap_or_default()
    }

    /// A crypto-free content signature of the raw registry document, so the validated map (one
    /// signature check per entry) is only rebuilt when the bytes changed.
    fn devices_signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let Some(doc) = self
            .sync
            .doc(DocType::Devices, DEVICES_DOC)
            .map(|d| d.doc())
        else {
            return 0;
        };
        for (key, map) in [(D_DEVICES, 0u8), (D_REVOCATIONS, 1u8)] {
            map.hash(&mut h);
            if let Ok(Some((Value::Object(ObjType::Map), obj))) = doc.get(ROOT, key) {
                for fp in doc.keys(&obj) {
                    fp.hash(&mut h);
                    if let Ok(Some((Value::Object(ObjType::Map), entry))) = doc.get(&obj, &fp) {
                        str_field(doc, &entry, D_ORIGIN).hash(&mut h);
                        str_field(doc, &entry, D_NAME).hash(&mut h);
                        bytes_field(doc, &entry, D_CERT).hash(&mut h);
                    }
                }
            }
        }
        h.finish()
    }

    /// Publish any companion this device just admitted into the shared registry, then push the
    /// re-validated companion → origin map and revocation set down to the sync layer, where the
    /// owner's depth-1 admission gate reads them.
    ///
    /// Runs on every tick for every member. The **write** half only ever fires on the owner;
    /// admission is owner-serialized, so nobody else has an admitted certificate to publish, and
    /// the document has exactly one writer. The **read** half is what keeps a joiner's
    /// attribution map current after a catch-up.
    async fn reconcile_devices(&mut self) {
        let admitted = self.sync.take_admitted_devices();
        if !admitted.is_empty() {
            // The owner may have admitted before anything opened the registry (a founder that
            // never called `open_devices`); opening is idempotent.
            if let Err(e) = self.open_devices().await {
                tracing::warn!(error = %e, "opening the device registry failed");
            }
            // The owner signs each entry so no other member can forge one (see D_OWNER_SIG).
            let group_id = self.sync.group_id();
            let owner_pk = self.sync.my_public_key();
            for cert in admitted {
                let fp = fingerprint(&cert.new_device_id);
                let payload = device_entry_payload(&group_id, &fp, &cert.encode());
                let owner_sig = match self.sync.sign_blob(&payload) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "signing a device-registry entry failed");
                        continue;
                    }
                };
                let owner_pk = owner_pk.clone();
                if let Err(e) = self
                    .sync
                    .post(DocType::Devices, DEVICES_DOC, move |d| {
                        write_device_entry(d, &cert, &owner_pk, &owner_sig)
                    })
                    .await
                {
                    tracing::warn!(error = %e, "recording an admitted companion device failed");
                }
            }
        }
        let sig = self.devices_signature();
        if self.devices_sig == Some(sig) {
            return; // unchanged; skip the per-entry signature checks
        }
        self.devices_sig = Some(sig);
        let group_id = self.sync.group_id();
        let owner_id = self.sync.designated_committer_id();
        let (companions, revoked) = match (self.sync.doc(DocType::Devices, DEVICES_DOC), owner_id) {
            (Some(d), Some(owner)) => {
                let doc = d.doc();
                let companions: HashMap<DeviceId, DeviceId> =
                    read_device_certs(doc, &group_id, &owner)
                        .into_iter()
                        .map(|c| (c.new_device_id, c.origin_id))
                        .collect();
                // A revocation counts only if its origin matches the companion's REGISTERED origin
                // (see `honored_revocations`); so member A cannot evict member B's device.
                let revoked = honored_revocations(&companions, &read_device_revocations(doc));
                (companions, revoked)
            }
            _ => (HashMap::new(), HashSet::new()),
        };
        // Owner enforcement (M5): a revoked companion that is still an MLS member gets its leaf
        // removed. Owner-serialized like every other admission/removal, so no fork. Re-admission
        // is already blocked by the consumed certificate ledger AND the revoked set.
        if self.is_owner() {
            let members: HashSet<DeviceId> = self.members().into_iter().collect();
            for dev in revoked
                .iter()
                .filter(|d| members.contains(d))
                .copied()
                .collect::<Vec<_>>()
            {
                if let Err(e) = self.sync.request_remove(&dev).await {
                    tracing::warn!(error = %e, "removing a revoked companion leaf failed");
                }
            }
        }
        self.sync.set_device_registry(companions, revoked);
    }

    /// Revoke one of **your own** companion devices (M5: the "lost phone" verb). Signs an
    /// origin-signed [`DeviceRevocation`] and publishes it; the owner enforces the MLS Remove when
    /// it next reconciles. Only the device's *origin* can call this; the revocation is signed with
    /// this device's key, and the owner only honours a revocation whose origin matches the
    /// companion's registered origin. (An owner kicking someone else's member/devices uses
    /// [`Server::remove_member`], which removes leaves directly by owner authority.)
    pub async fn revoke_device(&mut self, companion_fp: &str) -> Result<(), AppError> {
        let devices = self.devices();
        let entry = devices
            .get(companion_fp)
            .ok_or_else(|| AppError::Invalid("no such linked device".into()))?;
        if entry.origin != self.my_fingerprint() {
            return Err(AppError::Invalid(
                "only the device's own origin can revoke it".into(),
            ));
        }
        // Resolve the companion fingerprint to its full device id (unambiguously; the registry is
        // keyed by the full-fingerprint we compare against).
        let revoked_id = self
            .members()
            .into_iter()
            .find(|d| fingerprint(d) == companion_fp)
            .ok_or_else(|| AppError::Invalid("that device is not a current member".into()))?;
        let origin_id = self.device_id();
        let origin_pk: [u8; 32] = self
            .sync
            .my_public_key()
            .as_slice()
            .try_into()
            .map_err(|_| AppError::Invalid("origin key is not 32 bytes".into()))?;
        let now = self.sync.now_ms();
        let payload = DeviceRevocation::signing_payload(&origin_id, &origin_pk, &revoked_id, now);
        let signature = self.sync.sign_blob(&payload)?;
        let rev = DeviceRevocation {
            origin_id,
            origin_public_key: origin_pk,
            revoked_device_id: revoked_id,
            rev_ts_ms: now,
            signature,
        };
        self.sync
            .post(DocType::Devices, DEVICES_DOC, move |d| {
                write_revocation_entry(d, &rev)
            })
            .await?;
        Ok(())
    }

    // --- member roles / permissions (Phase 10h) ----------------------------

    /// Open (create/subscribe) the per-server member-roles document. Call once after
    /// founding/joining.
    pub async fn open_roles(&mut self) -> Result<(), AppError> {
        self.sync
            .open_channel(DocType::MemberRoles, ROLES_DOC)
            .await?;
        Ok(())
    }

    // --- moderation plane ---------------------------------------------------

    /// Subscribe to the server's signed moderation evidence/cases/votes document.
    pub async fn open_moderation(&mut self) -> Result<(), AppError> {
        self.sync
            .open_channel(DocType::Moderation, moderation::MODERATION_DOC)
            .await?;
        Ok(())
    }

    /// Pull moderation history from a known current member.
    pub async fn request_moderation_catchup(&mut self, peer: PeerId) -> Result<usize, AppError> {
        Ok(self
            .sync
            .request_catchup(peer, DocType::Moderation, moderation::MODERATION_DOC)
            .await?)
    }

    /// Materialize signed moderation records. Signature validity and current authority are
    /// separate by design: current role state cannot prove what role a signer held historically.
    pub fn moderation_state(&self) -> ModerationState {
        let Some(doc) = self
            .sync
            .doc(DocType::Moderation, moderation::MODERATION_DOC)
        else {
            return ModerationState::default();
        };
        let mut state = moderation::read_state(doc.doc(), &self.group_id());
        for event in &mut state.events {
            event.signature_valid &= self.origin_of(&event.signer) == event.actor;
            event.authorized = event.signature_valid
                && match event.kind.as_str() {
                    "warning" | "kick_case" => {
                        matches!(self.role_of(&event.actor), Role::Owner | Role::Admin)
                    }
                    "case_resolution" => self.role_of(&event.actor) == Role::Owner,
                    _ => false,
                };
        }
        let current_identities: HashSet<String> = self
            .members_view()
            .into_iter()
            .map(|member| self.origin_of(&member.fingerprint))
            .collect();
        for vote in &mut state.votes {
            vote.signature_valid &= self.origin_of(&vote.signer) == vote.voter;
            vote.eligible = vote.signature_valid && current_identities.contains(&vote.voter);
        }
        state
    }

    fn moderation_identity(&self) -> (String, String, Vec<u8>) {
        let signer = self.my_fingerprint();
        (self.origin_of(&signer), signer, self.sync.my_public_key())
    }

    async fn post_moderation_event(&mut self, event: ModerationEvent) -> Result<(), AppError> {
        let group = self.group_id();
        let event = moderation::sign_event(&group, event, |payload| {
            self.sync.sign_blob(payload).map_err(AppError::from)
        })?;
        self.sync
            .post(
                DocType::Moderation,
                moderation::MODERATION_DOC,
                move |doc| moderation::write_event(doc, &event),
            )
            .await?;
        Ok(())
    }

    /// Warn one currently-visible message, preserving the observed text as immutable evidence.
    pub async fn warn_message(
        &mut self,
        channel: u128,
        message_id: &str,
        reason: &str,
    ) -> Result<String, AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) {
            return Err(AppError::Invalid(
                "only an owner/admin can warn a message".into(),
            ));
        }
        let reason = reason.trim();
        if reason.is_empty() || reason.len() > MAX_MOD_REASON_BYTES {
            return Err(AppError::Invalid(format!(
                "warning reason must be 1..={MAX_MOD_REASON_BYTES} bytes"
            )));
        }
        let message = self
            .messages(channel)
            .into_iter()
            .find(|message| message.id == message_id)
            .ok_or_else(|| AppError::Invalid("no such message".into()))?;
        if message.text.len() > MAX_MOD_EVIDENCE_BYTES {
            return Err(AppError::Invalid(
                "message is too large to preserve as warning evidence".into(),
            ));
        }
        let (actor, signer, public_key) = self.moderation_identity();
        let id = self.sync.random_id();
        let event = ModerationEvent {
            id: id.clone(),
            kind: "warning".into(),
            actor,
            signer,
            target: self.origin_of(&message.author),
            channel: channel.to_string(),
            message_id: message.id,
            message_text: message.text,
            message_ts: message.ts,
            reason: reason.to_string(),
            ts: self.now_ms(),
            public_key,
            ..ModerationEvent::default()
        };
        self.post_moderation_event(event).await?;
        Ok(id)
    }

    /// Open a public, advisory kick case backed by warnings for the same target.
    pub async fn create_kick_case(
        &mut self,
        target: &str,
        reason: &str,
        evidence_ids: &[String],
    ) -> Result<String, AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) {
            return Err(AppError::Invalid(
                "only an owner/admin can open a kick case".into(),
            ));
        }
        let target = self.origin_of(target);
        let reason = reason.trim();
        if reason.is_empty() || reason.len() > MAX_MOD_REASON_BYTES {
            return Err(AppError::Invalid(format!(
                "case reason must be 1..={MAX_MOD_REASON_BYTES} bytes"
            )));
        }
        if evidence_ids.len() > MAX_MOD_EVIDENCE_IDS {
            return Err(AppError::Invalid("too many evidence items".into()));
        }
        let identities: HashSet<String> = self
            .members_view()
            .into_iter()
            .map(|member| self.origin_of(&member.fingerprint))
            .collect();
        if !identities.contains(&target) {
            return Err(AppError::Invalid(
                "case target is not a current member".into(),
            ));
        }
        if self.owner_fingerprint().as_deref() == Some(target.as_str()) {
            return Err(AppError::Invalid(
                "the owner cannot be a kick target".into(),
            ));
        }
        let current = self.moderation_state();
        for evidence_id in evidence_ids {
            let valid = current.events.iter().any(|event| {
                event.id == *evidence_id
                    && event.kind == "warning"
                    && event.target == target
                    && event.authorized
            });
            if !valid {
                return Err(AppError::Invalid(
                    "case evidence must be an authorized warning for the target".into(),
                ));
            }
        }
        let (actor, signer, public_key) = self.moderation_identity();
        let id = self.sync.random_id();
        let event = ModerationEvent {
            id: id.clone(),
            kind: "kick_case".into(),
            actor,
            signer,
            target,
            reason: reason.to_string(),
            evidence_ids: evidence_ids.to_vec(),
            ts: self.now_ms(),
            public_key,
            ..ModerationEvent::default()
        };
        self.post_moderation_event(event).await?;
        Ok(id)
    }

    /// Cast or replace this member identity's advisory vote. A linked device writes under its
    /// origin identity's key, so it cannot add a second vote.
    pub async fn cast_kick_vote(&mut self, case_id: &str, yes: bool) -> Result<(), AppError> {
        let state = self.moderation_state();
        if !moderation::case_is_open(&state.events, case_id) {
            return Err(AppError::Invalid("kick case is not open".into()));
        }
        let (voter, signer, public_key) = self.moderation_identity();
        let group = self.group_id();
        let vote = moderation::sign_vote(
            &group,
            ModerationVote {
                case_id: case_id.to_string(),
                voter,
                signer,
                yes,
                ts: self.now_ms(),
                public_key,
                ..ModerationVote::default()
            },
            |payload| self.sync.sign_blob(payload).map_err(AppError::from),
        )?;
        self.sync
            .post(
                DocType::Moderation,
                moderation::MODERATION_DOC,
                move |doc| moderation::write_vote(doc, &vote),
            )
            .await?;
        Ok(())
    }

    /// Owner decision on a kick case. `remove=true` invokes the existing protocol-enforced MLS
    /// removal; the signed resolution records success or failure afterward. Votes remain advisory.
    pub async fn resolve_kick_case(&mut self, case_id: &str, remove: bool) -> Result<(), AppError> {
        if !self.is_owner() {
            return Err(AppError::Invalid(
                "only the owner can resolve a kick case".into(),
            ));
        }
        let state = self.moderation_state();
        if !moderation::case_is_open(&state.events, case_id) {
            return Err(AppError::Invalid("kick case is not open".into()));
        }
        let case = state
            .events
            .iter()
            .find(|event| event.id == case_id && event.kind == "kick_case" && event.authorized)
            .cloned()
            .ok_or_else(|| AppError::Invalid("kick case is not authorized".into()))?;

        let removal_error = if remove {
            self.remove_member(&case.target).await.err()
        } else {
            None
        };
        let outcome = if !remove {
            "dismissed"
        } else if removal_error.is_none() {
            "removed"
        } else {
            "remove_failed"
        };
        let (actor, signer, public_key) = self.moderation_identity();
        let event = ModerationEvent {
            id: self.sync.random_id(),
            kind: "case_resolution".into(),
            actor,
            signer,
            target: case.target,
            case_id: case_id.to_string(),
            outcome: outcome.into(),
            ts: self.now_ms(),
            public_key,
            ..ModerationEvent::default()
        };
        self.post_moderation_event(event).await?;
        if let Some(error) = removal_error {
            return Err(error);
        }
        Ok(())
    }

    /// Catch up the roles document from `peer` (e.g. right after joining).
    pub async fn request_roles_catchup(&mut self, peer: PeerId) -> Result<usize, AppError> {
        Ok(self
            .sync
            .request_catchup(peer, DocType::MemberRoles, ROLES_DOC)
            .await?)
    }

    /// Whether **this** member is the server owner (the MLS designated committer / founder).
    pub fn is_owner(&self) -> bool {
        self.sync.is_designated_committer()
    }

    /// The owner's device fingerprint (the designated committer), if any.
    pub fn owner_fingerprint(&self) -> Option<String> {
        self.sync
            .designated_committer_id()
            .as_ref()
            .map(fingerprint)
    }

    /// The set of admin fingerprints, **filtered to current members** so a departed member does
    /// not resolve to a live admin. The **owner** displays its LOCAL authoritative roster (item 3
    ///; the same source the admission gate uses, so a tampered/replayed CRDT copy can't mislead
    /// the owner's own UI); other members read the owner-signed published copy.
    fn admin_set(&self) -> std::collections::HashSet<String> {
        let Some(owner_id) = self.sync.designated_committer_id() else {
            return std::collections::HashSet::new();
        };
        let members: std::collections::HashSet<String> = self
            .members_view()
            .into_iter()
            .map(|m| m.fingerprint)
            .collect();
        let admins = if self.is_owner() {
            self.sync.admin_roster()
        } else {
            let group_id = self.sync.group_id();
            self.sync
                .doc(DocType::MemberRoles, ROLES_DOC)
                .and_then(|d| read_published_roster(d.doc(), &group_id, &owner_id))
                .unwrap_or_default()
        };
        admins
            .into_iter()
            .filter(|fp| members.contains(fp))
            .collect()
    }

    /// This member's effective [`Role`].
    pub fn my_role(&self) -> Role {
        self.role_of(&self.my_fingerprint())
    }

    /// The effective [`Role`] of a member by fingerprint.
    pub fn role_of(&self, fp: &str) -> Role {
        if self.owner_fingerprint().as_deref() == Some(fp) {
            Role::Owner
        } else if self.admin_set().contains(fp) {
            Role::Admin
        } else {
            Role::Member
        }
    }

    /// Every known member's role, keyed by fingerprint (for the UI roster/settings).
    pub fn roles(&self) -> HashMap<String, String> {
        let owner = self.owner_fingerprint();
        let admins = self.admin_set();
        self.members_view()
            .into_iter()
            .map(|m| {
                let role = if owner.as_deref() == Some(m.fingerprint.as_str()) {
                    Role::Owner
                } else if admins.contains(&m.fingerprint) {
                    Role::Admin
                } else {
                    Role::Member
                };
                (m.fingerprint, role.as_str().to_string())
            })
            .collect()
    }

    /// Grant or revoke admin for a member fingerprint. **Owner only**; errors otherwise. Updates
    /// the owner's LOCAL authoritative roster (the admission source of truth, item 3) and publishes
    /// a fresh owner-signed copy into the roles doc for display; a demoted admin re-publishing its
    /// old roster into the CRDT cannot re-authorize itself because the gate reads the local set.
    pub async fn set_admin(&mut self, fp: &str, admin: bool) -> Result<(), AppError> {
        if !self.is_owner() {
            return Err(AppError::Invalid("only the owner can change roles".into()));
        }
        self.sync.set_admin(fp, admin).await?;
        Ok(())
    }

    /// Remove a member from the server by fingerprint. **Owner only**, enforced at *both* this
    /// product layer and the protocol layer: `request_remove` rejects a non-owner outright and
    /// the committer ignores any inbound remove request that isn't from the owner (THREAT-MODEL
    /// R1). The owner is the committer, so this removes directly; the MLS commit advances the
    /// epoch, so the removed member loses access to future content (forward secrecy) and the
    /// routing secret rotates.
    pub async fn remove_member(&mut self, fp: &str) -> Result<(), AppError> {
        if !self.is_owner() {
            return Err(AppError::Invalid(
                "only the owner can remove members".into(),
            ));
        }
        if fp == self.my_fingerprint() {
            return Err(AppError::Invalid("the owner cannot remove itself".into()));
        }
        // Resolve the fingerprint to exactly one member; reject an ambiguous 4-byte-prefix
        // collision so a removal can't be misdirected to an unintended same-prefix member.
        let mut matches = self.members().into_iter().filter(|d| fingerprint(d) == fp);
        let target = matches
            .next()
            .ok_or_else(|| AppError::Invalid("no such member".into()))?;
        if matches.next().is_some() {
            return Err(AppError::Invalid("ambiguous member fingerprint".into()));
        }
        // Cascade to the member's linked devices (M5): kicking a member removes their companion
        // leaves too, so a lingering device can't keep speaking for a removed member. Each is
        // owner-serialized like the origin's own removal.
        let members: HashSet<DeviceId> = self.members().into_iter().collect();
        for (comp, origin) in self.sync.companion_map() {
            if origin == target && members.contains(&comp) {
                if let Err(e) = self.sync.request_remove(&comp).await {
                    tracing::warn!(error = %e, "removing a kicked member's companion failed");
                }
            }
        }
        self.sync.request_remove(&target).await?;
        Ok(())
    }

    /// The current materialized messages in a channel (empty if it is not open).
    ///
    /// A clone of the cached list (see `messages_cache`); callers that only need to look use
    /// [`Self::with_messages`] and pay nothing per row.
    pub fn messages(&self, channel: u128) -> Vec<ChatMessage> {
        self.with_messages(channel, |msgs| msgs.to_vec())
    }

    /// Run `f` over the channel's current messages without copying them. The list is materialized
    /// at most once per document version; every later call at the same version borrows it.
    ///
    /// `f` must not call back into a message read of this server (the cache is borrowed while it
    /// runs); the projections that use this only read the slice they are handed.
    pub fn with_messages<U>(&self, channel: u128, f: impl FnOnce(&[ChatMessage]) -> U) -> U {
        self.with_channel(channel, |c| f(&c.messages))
    }

    /// As [`Self::with_messages`], with the by-id and reply-count lookups alongside the rows.
    fn with_channel<U>(&self, channel: u128, f: impl FnOnce(&MaterializedChannel) -> U) -> U {
        let Some(doc) = self.sync.doc(DocType::Channel, channel) else {
            // Not open: nothing to cache, and a stale entry from before a re-open is unreachable
            // since a document is never removed from the sync layer within a session.
            return f(&MaterializedChannel {
                version: 0,
                ..MaterializedChannel::default()
            });
        };
        let version = doc.op_count() as u64;
        let mut cache = self.messages_cache.borrow_mut();
        let entry = cache.entry(channel).or_insert_with(|| MaterializedChannel {
            version: u64::MAX,
            ..MaterializedChannel::default()
        });
        if entry.version != version {
            *entry = MaterializedChannel::build(version, read_messages(doc.doc()));
        }
        f(entry)
    }

    /// A bounded, contiguous slice of a channel around an anchor (see [`MessagePageQuery`]), with
    /// each row's cross-channel needs (mention/reply addressing, reply count, reply parent)
    /// resolved here against the whole materialized list. This is what lets the webview hold only
    /// the rows it shows: an id anchor survives concurrent inserts, edits and deletes around it,
    /// and `version` tells a caller whether two pages describe the same list.
    pub fn message_page(&self, channel: u128, query: &MessagePageQuery) -> MessagePage {
        let me = self.my_fingerprint();
        let marker = self.mention_marker();
        self.with_channel(channel, |c| {
            let total = c.messages.len();
            let mut page = MessagePage {
                version: c.version,
                total: total as u64,
                ..MessagePage::default()
            };
            if let Some(probe) = &query.unread {
                page.unread = Some(unread_summary(&c.messages, &me, probe));
            }
            let anchor = match &query.anchor {
                _ if total == 0 => None,
                PageAnchor::Tail => Some(total - 1),
                PageAnchor::Id(id) => c.by_id.get(id).copied(),
                PageAnchor::Index(index) => Some((*index as usize).min(total - 1)),
                PageAnchor::FirstReplyTo(id) => c.messages.iter().position(|m| &m.reply_to == id),
            };
            let Some(anchor) = anchor else {
                return page;
            };
            let start = anchor.saturating_sub(query.before);
            let end = anchor.saturating_add(query.after).min(total - 1);
            page.anchor_index = Some(anchor as u64);
            page.start = start as u64;
            page.rows = c.messages[start..=end]
                .iter()
                .map(|m| {
                    let parent = (!m.reply_to.is_empty())
                        .then(|| c.by_id.get(&m.reply_to).map(|&i| &c.messages[i]))
                        .flatten();
                    let targets_me = m.author != me
                        && (marker.as_deref().is_some_and(|mk| m.text.contains(mk))
                            || parent.is_some_and(|p| p.author == me));
                    PagedMessage {
                        message: m.clone(),
                        targets_me,
                        reply_count: c.reply_counts.get(&m.id).copied().unwrap_or(0),
                        reply_to_preview: parent.map(|p| ReplyPreview {
                            id: p.id.clone(),
                            author: p.author.clone(),
                            text: p.text.chars().take(REPLY_PREVIEW_CHARS).collect(),
                        }),
                    }
                })
                .collect();
            page
        })
    }

    /// Every pinned message in a channel, in log order. Pins are few and owner/admin-curated, so
    /// this is the one whole-channel projection a paged client still asks for by name.
    pub fn pinned_messages(&self, channel: u128) -> Vec<ChatMessage> {
        self.with_messages(channel, |msgs| {
            msgs.iter().filter(|m| m.pinned).cloned().collect()
        })
    }

    /// One activity head per known channel: what the newest message is, and what the newest
    /// message somebody ELSE wrote is, with no text attached.
    ///
    /// This is the projection unread state is rebuilt from. A live `ChannelUpdated` stream only
    /// reports what happened while the UI was listening, so an explicit lock, a restart or an
    /// offline catch-up all lose outstanding indicators; comparing these heads against the
    /// client's durable read marks recovers them from state rather than from event history.
    pub fn channel_heads(&self) -> Vec<ChannelHead> {
        let me = self.my_fingerprint();
        self.channels()
            .into_iter()
            .map(|c| {
                self.with_messages(c.id, |msgs| {
                    let mut head = ChannelHead {
                        channel: c.id,
                        count: msgs.len() as u64,
                        ..ChannelHead::default()
                    };
                    for m in msgs {
                        head.latest_ts = head.latest_ts.max(m.ts);
                        // `>=` so that among equal timestamps the last one materialized wins,
                        // which is the same row the UI shows at the bottom of the log.
                        if m.author != me && m.ts >= head.latest_incoming_ts {
                            head.latest_incoming_ts = m.ts;
                            head.latest_incoming_id = m.id.clone();
                        }
                    }
                    head
                })
            })
            .collect()
    }

    /// The newest `limit` messages of a channel, oldest first, each paired with whether it is
    /// addressed to me: an `@[my name]` mention (the composer's marker, under the same
    /// normalization [`Self::inbox`] uses) or a reply to one of my own messages. The reply parent
    /// is resolved against the whole channel, so a reply to something far older than the tail is
    /// still recognised; only the rows shipped are bounded. My own messages are never "to me".
    ///
    /// This is what an arrival notification needs, and all it needs: the previous shape of that
    /// path fetched the entire history of a channel nobody was looking at to read its last row.
    pub fn message_tail(&self, channel: u128, limit: usize) -> Vec<(ChatMessage, bool)> {
        let query = MessagePageQuery {
            anchor: PageAnchor::Tail,
            before: limit.saturating_sub(1),
            after: 0,
            unread: None,
        };
        self.message_page(channel, &query)
            .rows
            .into_iter()
            .map(|row| (row.message, row.targets_me))
            .collect()
    }

    /// The `@[Name]` marker the composer inserts for me, or `None` when I have no usable name.
    /// Built from the SAME normalization the composer applies (desktop `mentionName`), so a name
    /// with brackets/newlines/extra spaces or over the length cap is detected here exactly as it
    /// was written; not silently missed.
    fn mention_marker(&self) -> Option<String> {
        let me = self.my_fingerprint();
        let name = self
            .profiles()
            .get(&me)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let my_name = normalize_mention_name(&name);
        (!my_name.is_empty()).then(|| format!("@[{my_name}]"))
    }

    /// Lightweight activity stats over a channel's messages; total count, first/last wall-clock
    /// timestamp, and the number of distinct days a message was sent; for the friends-list
    /// sortings (activity / reconnect / recency) WITHOUT shipping message text to the UI.
    pub fn message_stats(&self, channel: u128) -> MessageStats {
        let msgs = self.messages(channel);
        let mut days = std::collections::HashSet::new();
        let mut first = u64::MAX;
        let mut last = 0u64;
        for m in &msgs {
            if m.ts == 0 {
                continue; // a legacy message with no timestamp doesn't anchor a day
            }
            first = first.min(m.ts);
            last = last.max(m.ts);
            days.insert(m.ts / 86_400_000); // bucket by UTC day
        }
        MessageStats {
            count: msgs.len() as u64,
            first_ts: if first == u64::MAX { 0 } else { first },
            last_ts: last,
            active_days: days.len() as u64,
        }
    }

    /// Scan every open channel for messages addressed to me; either an `@[my name]` mention (the
    /// UI's mention marker) or a reply to one of my own messages; newest first, capped at `limit`.
    /// Author names are resolved here because they are per-server. Excludes my own messages.
    pub fn inbox(&self, limit: usize) -> Vec<InboxItem> {
        let me = self.my_fingerprint();
        let names: std::collections::HashMap<String, String> = self
            .profiles()
            .into_iter()
            .map(|(fp, p)| (fp, p.name))
            .collect();
        let marker = self.mention_marker();

        let mut out = Vec::new();
        for channel in self.sync.channel_ids() {
            self.with_messages(channel, |msgs| {
                // message id -> author, to resolve a reply's parent within this channel.
                let author_of: std::collections::HashMap<&str, &str> = msgs
                    .iter()
                    .filter(|m| !m.id.is_empty())
                    .map(|m| (m.id.as_str(), m.author.as_str()))
                    .collect();
                for m in msgs {
                    if m.author == me {
                        continue; // never inbox my own messages
                    }
                    let mention = marker.as_deref().is_some_and(|mk| m.text.contains(mk));
                    let reply = !m.reply_to.is_empty()
                        && author_of.get(m.reply_to.as_str()) == Some(&me.as_str());
                    if mention || reply {
                        out.push(InboxItem {
                            channel,
                            message_id: m.id.clone(),
                            author: m.author.clone(),
                            author_name: names.get(&m.author).cloned().unwrap_or_default(),
                            text: m.text.clone(),
                            ts: m.ts,
                            mention,
                            reply,
                        });
                    }
                }
            });
        }
        out.sort_by(|a, b| b.ts.cmp(&a.ts));
        out.truncate(limit);
        out
    }

    /// Mint a single-use invite to this server.
    ///
    /// Minting does **not** lift outstanding transport evictions; see
    /// [`Server::readmit_evicted_peers`] for why that is a separate, deliberate action.
    pub fn mint_invite(
        &self,
        nonce: [u8; 16],
        expires_at_ms: u64,
        bootstrap: Vec<String>,
    ) -> Result<InviteToken, AppError> {
        self.require_invite_permission()?;
        Ok(self.sync.mint_invite(nonce, expires_at_ms, bootstrap)?)
    }

    /// Lift every outstanding transport eviction (P6), so a member removed earlier can reach
    /// this node again to redeem an invite.
    ///
    /// This is the **deliberate** half of remove-then-re-invite and must be wired only to an
    /// explicit user action ("Generate new invite"), never to a path that can run on its own.
    /// The desktop re-mints an invite automatically whenever the node gains an address the
    /// stored invite does not mention, so folding this into minting would silently re-admit
    /// every removed member the next time anybody opened the invite panel. See
    /// `ChannelSync::lift_all_evictions`.
    ///
    /// Owner/admin only, the same gate as minting: it is the same declaration of intent.
    /// It cannot be narrower than "everyone currently evicted", because an `InviteToken` names
    /// no invitee.
    pub fn readmit_evicted_peers(&mut self) -> Result<(), AppError> {
        self.require_invite_permission()?;
        self.sync.lift_all_evictions();
        Ok(())
    }

    /// Gate on the caller being owner/admin (Phase 10h). Policy-layer: enforced for honest
    /// clients + the UI; the committer-side join-time re-check is the hardening follow-up.
    fn require_invite_permission(&self) -> Result<(), AppError> {
        if self.my_role().can_invite() {
            Ok(())
        } else {
            Err(AppError::Invalid(
                "only the owner or an admin can invite to this server".into(),
            ))
        }
    }

    /// Mint an invite that also carries rendezvous infra addresses (discovery-enabled).
    pub fn mint_invite_with_rendezvous(
        &self,
        nonce: [u8; 16],
        expires_at_ms: u64,
        bootstrap: Vec<String>,
        rendezvous: Vec<String>,
    ) -> Result<InviteToken, AppError> {
        self.require_invite_permission()?;
        Ok(self
            .sync
            .mint_invite_with_rendezvous(nonce, expires_at_ms, bootstrap, rendezvous)?)
    }

    /// The roster; device ids of all current members.
    pub fn members(&self) -> Vec<DeviceId> {
        self.sync.member_ids()
    }

    /// The roster as UI-facing [`MemberView`]s (fingerprint + whether it is this device).
    pub fn members_view(&self) -> Vec<MemberView> {
        let me = self.device_id;
        self.sync
            .member_ids()
            .into_iter()
            .map(|id| MemberView {
                fingerprint: fingerprint(&id),
                identity: hex::encode(id.as_bytes()),
                is_self: id == me,
            })
            .collect()
    }

    /// This device's content-addressed id.
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// The current member count.
    pub fn member_count(&self) -> usize {
        self.sync.member_count()
    }

    /// The current MLS epoch.
    pub fn epoch(&self) -> u64 {
        self.sync.epoch()
    }

    /// This device's transport peer id.
    pub fn local_peer(&self) -> PeerId {
        self.sync.local_peer()
    }

    /// This server's chosen display name.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Advance the server: process one inbound network event (after draining the outbox
    /// and any recovery). Returns `false` once the transport has closed. The bridge
    /// layer drives this in a background loop; tests drive it explicitly.
    pub async fn sync_once(&mut self) -> Result<bool, AppError> {
        let cont = self.sync.run_once().await?;
        // Publish anything the tick admitted, and refresh the companion → origin registry the
        // sync layer's depth-1 admission gate reads (multi-device M3).
        self.reconcile_devices().await;
        Ok(cont)
    }

    // --- steady-state rendezvous discovery (driven by the actor) -----------------------------

    /// Configure the rendezvous nodes this member registers/discovers at, so the group re-finds
    /// itself after a restart (founder: the rendezvous it registered at; joiner: the invite's).
    pub fn set_rendezvous_nodes(&mut self, nodes: Vec<(String, Vec<u8>)>) {
        self.sync.set_rendezvous_nodes(nodes);
    }

    /// Share one process-level endpoint budget across this server and every sibling group actor.
    /// This must be installed before eager cached redial on restore.
    pub fn set_endpoint_dial_scheduler(
        &mut self,
        scheduler: catcoms_discovery::EndpointDialScheduler,
    ) {
        self.sync.set_endpoint_dial_scheduler(scheduler);
    }

    /// Install this device's sealed, Noise-authenticated direct reconnect hints. Private routes
    /// stay local and are never folded into peer exchange or the shared address cache.
    pub fn set_local_reconnect_routes(&mut self, routes: Vec<(PeerId, String)>) {
        self.sync.set_local_reconnect_routes(routes);
    }

    /// Retry sealed local reconnect hints after re-checking the current roster, exact peer
    /// binding, canonical route grammar and shared endpoint budget.
    pub async fn dial_local_reconnect_routes(&mut self) -> usize {
        self.sync.dial_local_reconnect_routes().await
    }

    /// Produce this existing member's short-lived, signed out-of-band route update.
    pub fn mint_member_recovery_code(
        &mut self,
        candidates: Vec<String>,
    ) -> Result<MemberRecoveryCode, AppError> {
        self.sync
            .mint_member_recovery_code(candidates)
            .map_err(|error| AppError::Invalid(error.to_string()))
    }

    /// Authenticate a recovery code without starting network work. The desktop uses this phase
    /// to durably record the short-lived peer permission before any submitted dial can complete.
    pub fn verify_member_recovery_code(
        &self,
        code: &str,
    ) -> Result<MemberRecoveryVerified, AppError> {
        self.sync
            .verify_member_recovery_code(code)
            .map_err(|error| AppError::Invalid(error.to_string()))
    }

    /// Verify a current member's signed route update and submit its bounded peer-bound dial.
    pub async fn apply_member_recovery_code(
        &mut self,
        code: &str,
    ) -> Result<MemberRecoveryApplied, AppError> {
        self.sync
            .apply_member_recovery_code(code)
            .await
            .map_err(|error| AppError::Invalid(error.to_string()))
    }

    /// Whether steady-state rendezvous discovery is configured (so the actor drives its tick).
    pub fn has_rendezvous(&self) -> bool {
        self.sync.has_rendezvous()
    }

    /// Drive one steady-state discovery tick (re-register + re-discover at the rendezvous).
    pub async fn drive_discovery(&mut self) {
        self.sync.drive_discovery().await;
    }

    /// Await the next discovered record or registration TTL grant.
    pub async fn next_postjoin_discovery_event(
        &mut self,
    ) -> Option<catcoms_sync::PostJoinDiscoveryEvent> {
        self.sync.next_postjoin_discovery_event().await
    }

    /// Apply one registration TTL grant to the renewal scheduler.
    pub fn note_rendezvous_registered(
        &mut self,
        registration: catcoms_rt::RendezvousRegistration,
    ) -> bool {
        self.sync.note_rendezvous_registered(registration)
    }

    /// Dial a discovered peer if the [`DiscoveryPolicy`](catcoms_discovery::DiscoveryPolicy)
    /// approves it (never auto-dial; membership re-proven post-dial via PEX).
    pub async fn ingest_discovered(&mut self, d: DiscoveredPeer) {
        self.sync.ingest_discovered(d).await;
    }

    /// Advisory eclipse check; `true` if the node may be isolated (verify a member out of band).
    /// Never gates anything; the actor surfaces a changed verdict to the UI.
    pub fn observe_eclipse(&mut self) -> bool {
        self.sync.observe_eclipse()
    }

    // --- member peer exchange (PEX) + the cross-session address cache -------------------------

    /// Publish this device's own signed peer record, so other members can learn where to dial it.
    ///
    /// This is the **root of the whole steady-state discovery path** and until now nothing in the
    /// product called it, which left `peer_records` permanently empty and took presence, the
    /// cross-session re-dial and the eclipse detector's reach term down with it. Call it at
    /// found, join and reload with the addresses this node is actually reachable on.
    ///
    /// `seq` must come from the per-launch block reserved by
    /// [`ServerNet::reserve_record_seq_block`](crate::store::ServerNet::reserve_record_seq_block),
    /// never from a counter that restarts at 1: a peer keeps an incoming record only when its
    /// `seq` beats the one it already holds, so a restart that reuses low numbers is ignored
    /// forever by everyone holding the old record.
    pub fn publish_self_record(
        &mut self,
        addresses: Vec<String>,
        seq: u64,
    ) -> Result<(), AppError> {
        Ok(self.sync.publish_self_record(addresses, seq)?)
    }

    /// Refresh a connected member's separate, backward-compatible switchboard offer.
    pub async fn request_switchboard_offer(&mut self, peer: PeerId) -> Result<bool, AppError> {
        Ok(self.sync.request_switchboard_offer(peer).await?)
    }

    pub fn connected_switchboard_offers(&mut self) -> Vec<SwitchboardOffer> {
        self.sync.connected_switchboard_offers()
    }

    pub fn wrap_invite_with_switchboards(
        &mut self,
        invite_bytes: &[u8],
    ) -> Result<Vec<u8>, AppError> {
        Ok(self.sync.wrap_invite_with_switchboards(invite_bytes)?)
    }

    /// Ask a bounded handful of known peers for their member records (one PEX pass). Returns the
    /// number of newly-known members. Used by tests and by callers with no runtime to bound
    /// individual requests with; the actor uses [`Server::pex_targets`] + [`Server::request_pex`]
    /// so it can put a deadline on each request rather than on the whole pass.
    pub async fn drive_pex(&mut self) -> usize {
        self.sync.drive_pex().await
    }

    /// Choose this pass's PEX targets (and charge them against the requester-side rate limit).
    pub fn pex_targets(&mut self) -> Vec<PeerId> {
        self.sync.take_pex_targets()
    }

    /// Ask one peer for its member records. Returns the number of newly-known members.
    pub async fn request_pex(&mut self, peer: PeerId) -> Result<usize, AppError> {
        Ok(self.sync.request_pex(peer).await?)
    }

    /// Back a peer off after it failed to answer a PEX request within the caller's deadline.
    pub fn note_pex_failure(&mut self, peer: PeerId) {
        self.sync.note_pex_failure(peer);
    }

    /// Fold the currently-known member records into the cross-session address cache. Returns the
    /// cache size.
    pub fn cache_known_records(&mut self) -> usize {
        self.sync.cache_known_records()
    }

    /// Dial the cached previously-proven members (policy-gated; candidates only, never promoted
    /// into the trusted catch-up pool). Returns the number of peers dialed.
    pub async fn dial_cached_peers(&mut self) -> usize {
        self.sync.dial_cached_peers().await
    }

    /// Run one bounded SWIM/reciprocal/topology repair pass.
    pub async fn drive_mesh_repair(&mut self) -> usize {
        self.sync.drive_mesh_repair().await
    }

    pub fn has_pending_reciprocal(&self) -> bool {
        self.sync.has_pending_reciprocal()
    }

    pub async fn drive_pending_reciprocal(&mut self) -> usize {
        self.sync.drive_pending_reciprocal().await
    }

    /// Explicit fallback redial that preserves every shared safety budget.
    pub async fn manual_fallback_redial(&mut self) -> catcoms_sync::ManualRedialOutcome {
        self.sync.manual_fallback_redial().await
    }

    /// Serialize the address cache for sealing beside this server's snapshot.
    /// `integrity_key` comes from [`ServerStore::address_cache_key`](crate::store::ServerStore::address_cache_key).
    pub fn address_cache_bytes(&self, integrity_key: &[u8; 32]) -> Vec<u8> {
        self.sync.address_cache_bytes(integrity_key)
    }

    /// Load a previously sealed address cache. `false` if it failed its integrity tag or was
    /// malformed, in which case the node simply starts with no cached candidates.
    pub fn load_address_cache(&mut self, bytes: &[u8], integrity_key: &[u8; 32]) -> bool {
        self.sync.load_address_cache(bytes, integrity_key)
    }

    /// How many previously-proven members the cross-session cache is holding.
    pub fn cached_peer_count(&self) -> usize {
        self.sync.cached_peer_count()
    }

    /// Fetch a channel's history from `peer` (request/response catch-up), e.g. right
    /// after joining. Returns the number of newly-applied messages.
    pub async fn request_channel_catchup(
        &mut self,
        peer: PeerId,
        channel: u128,
    ) -> Result<usize, AppError> {
        Ok(self
            .sync
            .request_catchup(peer, DocType::Channel, channel)
            .await?)
    }

    /// Catch a channel up from the **best known peer** (a proven member, else any known
    /// peer); no need to name a peer. Lets either side pull the backlog of a channel the
    /// other created. Returns `Ok(0)` if there is no peer to ask yet.
    pub async fn request_channel_catchup_any(&mut self, channel: u128) -> Result<usize, AppError> {
        Ok(self
            .sync
            .request_catchup_best(DocType::Channel, channel)
            .await?)
    }

    /// Borrow the underlying [`ChannelSync`] (for the bridge layer / advanced use such
    /// as rendezvous registration via its transport).
    pub fn sync(&self) -> &ChannelSync<T, R> {
        &self.sync
    }
}

impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for Server<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("display_name", &self.display_name)
            .field("epoch", &self.sync.epoch())
            .field("members", &self.sync.member_count())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    const GENERAL: u128 = 1;

    /// The cursor walk must see exactly what the indexed walk saw, for rows written locally AND
    /// rows that arrived as another actor's change (the replication path is `load_incremental`,
    /// which is where a cursor over the op set could diverge from an indexed lookup).
    #[test]
    fn cursor_materialization_matches_indexed_lookup_for_remote_changes() {
        let mut alice = AutoCommit::new();
        alice.set_actor(automerge::ActorId::from(vec![1u8; 32]));
        append_message(&mut alice, "m1", "alice", "first", 10, "").unwrap();
        alice.commit();
        let first = alice.get_last_local_change().unwrap().raw_bytes().to_vec();

        let mut bob = AutoCommit::new();
        bob.set_actor(automerge::ActorId::from(vec![2u8; 32]));
        bob.load_incremental(&first).unwrap();
        append_message(&mut bob, "m2", "bob", "second", 11, "m1").unwrap();
        toggle_reaction_in_doc(&mut bob, "m1", "👍", "bob").unwrap();
        bob.commit();
        let second = bob.get_last_local_change().unwrap().raw_bytes().to_vec();
        alice.load_incremental(&second).unwrap();

        // The reference: the indexed walk this crate used before the cursor.
        fn indexed(doc: &AutoCommit) -> Vec<ChatMessage> {
            let mut out = Vec::new();
            if let Ok(Some((Value::Object(ObjType::List), list))) = doc.get(ROOT, MESSAGES) {
                for i in 0..doc.length(&list) {
                    if let Ok(Some((Value::Object(ObjType::Map), msg))) = doc.get(&list, i) {
                        let mut by = Vec::new();
                        for key in doc.keys(&msg) {
                            if let Some((emoji, fp)) = key.split_once(REACTION_SEP) {
                                by.push((emoji.to_string(), fp.to_string()));
                            }
                        }
                        out.push(ChatMessage {
                            id: str_field(doc, &msg, MSG_ID),
                            author: str_field(doc, &msg, AUTHOR),
                            text: str_field(doc, &msg, TEXT),
                            ts: int_field(doc, &msg, TS),
                            edited: int_field(doc, &msg, EDITED),
                            reactions: by
                                .into_iter()
                                .map(|(emoji, fp)| Reaction {
                                    emoji,
                                    by: vec![fp],
                                })
                                .collect(),
                            reply_to: str_field(doc, &msg, REPLY_TO),
                            pinned: doc.get(&msg, PINNED).ok().flatten().is_some(),
                        });
                    }
                }
            }
            out
        }
        for doc in [&alice, &bob] {
            let rows = read_messages(doc);
            assert_eq!(rows, indexed(doc));
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].reactions.len(), 1, "the remote reaction is visible");
            assert_eq!(rows[1].reply_to, "m1");
        }
    }

    /// Deterministic transport identities keep product-layer fixtures honest about the
    /// canonical `/p2p/<PeerId>` route binding enforced by steady-state discovery.
    fn test_libp2p_peer(n: u8) -> libp2p::PeerId {
        libp2p::identity::Keypair::ed25519_from_bytes([n; 32])
            .unwrap()
            .public()
            .to_peer_id()
    }

    fn test_transport_peer(n: u8) -> PeerId {
        catcoms_net::phase0_peer_id(&test_libp2p_peer(n))
    }

    fn test_peer_route(n: u8, base: &str) -> String {
        format!("{base}/p2p/{}", test_libp2p_peer(n))
    }

    /// Let `s` apply everything currently queued for it, then stop.
    ///
    /// `sync_once` ends in `transport.next_event().await`, which yields `None` only once the
    /// transport is *closed*: on an idle inbox it waits forever. So a fixed `for _ in 0..n` tick
    /// count is not "sync a few times", it is a bet that at least `n` events are already queued,
    /// and losing the bet hangs the test instead of failing it. Bound each tick instead and stop
    /// at the first one with nothing to do. If the bound ever proves too short the test fails an
    /// assertion, which is the failure mode we want: visible, not a wedged CI job.
    async fn drain_sync(s: &mut Server<MemNetwork, ChaCha20Rng>) {
        while let Ok(r) =
            tokio::time::timeout(std::time::Duration::from_millis(500), s.sync_once()).await
        {
            r.unwrap();
        }
    }

    fn founder() -> Server<MemNetwork, ChaCha20Rng> {
        let hub = Hub::new();
        Server::found(
            hub.join(PeerId::from_u64(1)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
            "alice",
        )
        .unwrap()
    }

    // ---------- multi-device: companion admission by certificate (M3) ----------
    //
    // The founder is the owner *and* the origin here (a two-member group), which is the shape
    // the design's happy path allows: the member pairing a second device is the same member that
    // serializes admissions. The certificate path is identical for any other member; only the
    // relay hop differs, and that is the already-reviewed admin-invite machinery.

    /// Time the M3 tests start at, well past any freshness horizon so `abs_diff` is meaningful.
    const T0: u64 = 1_700_000_000_000;
    /// `MAX_DEVICE_CERT_AGE_MS` in `catcoms-sync`; the window a certificate may be admitted in.
    const CERT_AGE: u64 = 3_600_000;

    /// A founder over `hub` whose clock the test controls.
    fn founder_on(
        hub: &std::sync::Arc<Hub>,
        peer: PeerId,
        clock: &ManualClock,
        seed: u64,
    ) -> Server<MemNetwork, ChaCha20Rng> {
        Server::found(
            hub.join(peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(seed),
            Box::new(clock.clone()),
            "alice",
        )
        .unwrap()
    }

    /// The grant a member's origin device mints for one companion; exactly the object
    /// `pairing_mint` assembles per server, with a certificate this server actually signed.
    fn grant_from(
        server: &Server<MemNetwork, ChaCha20Rng>,
        device: DeviceId,
        name: &str,
    ) -> PerServerGrant {
        PerServerGrant {
            group_id: server.group_id(),
            server_name: "cat cafe".into(),
            bootstrap: Vec::new(),
            rendezvous: Vec::new(),
            turn: String::new(),
            owner_public_key: server.owner_public_key().expect("the founder is the owner"),
            certificate: server
                .issue_device_certificate(device, name)
                .expect("origin signs a certificate"),
        }
    }

    /// A grant whose certificate came from somewhere other than this server's origin.
    fn grant_with(
        server: &Server<MemNetwork, ChaCha20Rng>,
        certificate: DeviceCertificate,
    ) -> PerServerGrant {
        PerServerGrant {
            group_id: server.group_id(),
            server_name: "cat cafe".into(),
            bootstrap: Vec::new(),
            rendezvous: Vec::new(),
            turn: String::new(),
            owner_public_key: server.owner_public_key().expect("the founder is the owner"),
            certificate,
        }
    }

    /// Present `grant` from a fresh companion device while `owner` serves the admission.
    ///
    /// The owner is ticked in a `select!` loop rather than `join!`ed, because a rejection the
    /// *presenting* device makes locally (a certificate that does not verify, or is not for this
    /// device) never reaches the wire; so there would be no event for the owner's tick to
    /// consume and a `join!` would simply hang.
    async fn present(
        hub: &std::sync::Arc<Hub>,
        peer: u64,
        owner_peer: PeerId,
        owner: &mut Server<MemNetwork, ChaCha20Rng>,
        clock: &ManualClock,
        device: MlsDevice,
        grant: &PerServerGrant,
    ) -> Result<Server<MemNetwork, ChaCha20Rng>, AppError> {
        let joining = Server::join_with_grant(
            hub.join(PeerId::from_u64(peer)),
            device,
            ChaCha20Rng::seed_from_u64(peer),
            Box::new(clock.clone()),
            "phone",
            owner_peer,
            grant,
        );
        let mut joining = std::pin::pin!(joining);
        loop {
            tokio::select! {
                joined = &mut joining => return joined,
                _ = owner.sync_once() => {}
            }
        }
    }

    #[tokio::test]
    async fn a_companion_device_joins_by_grant_and_is_attributable() {
        let hub = Hub::new();
        let clock = ManualClock::new(T0);
        let alice_peer = PeerId::from_u64(1);
        let mut alice = founder_on(&hub, alice_peer, &clock, 1);
        alice.subscribe_control().await.unwrap();
        alice.open_channel(GENERAL).await.unwrap();
        alice.open_devices().await.unwrap();
        assert!(alice.devices().is_empty(), "no companions yet");

        // The M2 ceremony's output, on the two devices: a fresh device key here, and a grant the
        // origin signed for exactly that key over there.
        let (secrets, _blob) = begin_pairing(&mut ChaCha20Rng::seed_from_u64(9)).unwrap();
        let phone_id = secrets.device_id();
        let grant = grant_from(&alice, phone_id, "phone");

        let mut phone = present(
            &hub,
            2,
            alice_peer,
            &mut alice,
            &clock,
            secrets.device().duplicate().unwrap(),
            &grant,
        )
        .await
        .expect("the origin's certificate admits the companion");

        // The companion is a real member with its own leaf; not a second holder of one key.
        assert_eq!(phone.member_count(), 2);
        assert_eq!(alice.member_count(), 2);
        assert!(alice.members().contains(&phone_id));
        assert_ne!(phone.device_id(), alice.device_id());

        // …and the owner published it into the `Devices` registry as part of admitting it.
        let phone_fp = fingerprint(&phone_id);
        let devices = alice.devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[&phone_fp].origin, alice.my_fingerprint());
        assert_eq!(devices[&phone_fp].name, "phone");

        // Attribution (what M4 renders): a companion's ops resolve to the member's origin
        // identity, and anything not in the registry is its own identity.
        assert_eq!(alice.origin_of(&phone_fp), alice.my_fingerprint());
        assert_eq!(alice.origin_of("deadbeef"), "deadbeef");

        // A message from the companion is authored by the *device* and attributable to the member.
        phone.open_channel(GENERAL).await.unwrap();
        phone.send_message(GENERAL, "from my phone").await.unwrap();
        assert!(alice.sync_once().await.unwrap());
        let msg = alice
            .messages(GENERAL)
            .into_iter()
            .find(|m| m.text == "from my phone")
            .expect("the companion's message reached the origin");
        assert_eq!(msg.author, phone_fp, "signed by the companion's own key");
        assert_eq!(
            alice.origin_of(&msg.author),
            alice.my_fingerprint(),
            "and renders under the member's identity"
        );
    }

    #[test]
    fn a_revocation_only_counts_against_its_own_origins_device() {
        // M5 security core: member A must not be able to evict member B's companion. A revocation
        // is honoured only when its origin matches the companion's registered origin.
        let a = catcoms_crypto::DeviceKeypair::generate(&mut ChaCha20Rng::seed_from_u64(1));
        let b = catcoms_crypto::DeviceKeypair::generate(&mut ChaCha20Rng::seed_from_u64(2));
        let a_phone =
            catcoms_crypto::DeviceKeypair::generate(&mut ChaCha20Rng::seed_from_u64(3)).device_id();
        let b_phone =
            catcoms_crypto::DeviceKeypair::generate(&mut ChaCha20Rng::seed_from_u64(4)).device_id();
        // Registry: A's phone belongs to A, B's phone belongs to B.
        let companions: HashMap<DeviceId, DeviceId> =
            HashMap::from([(a_phone, a.device_id()), (b_phone, b.device_id())]);

        // A validly revokes its OWN phone → honoured.
        let good = DeviceRevocation::issue(&a, a_phone, T0);
        assert_eq!(
            honored_revocations(&companions, &[good]),
            HashSet::from([a_phone])
        );

        // A signs a (syntactically valid) revocation naming B's phone → NOT honoured: A's origin id
        // is not B's phone's registered origin.
        let attack = DeviceRevocation::issue(&a, b_phone, T0);
        assert!(honored_revocations(&companions, &[attack]).is_empty());

        // B revoking B's own phone works, and a revocation of an unknown device is ignored.
        let b_ok = DeviceRevocation::issue(&b, b_phone, T0);
        let unknown = DeviceRevocation::issue(&a, a.device_id(), T0); // not in the registry
        assert_eq!(
            honored_revocations(&companions, &[b_ok, unknown]),
            HashSet::from([b_phone])
        );
    }

    // NOTE: a full owner-enforces-removal round-trip over the single-node facade was flaky in the
    // harness (the `sync_once`/self-delivered-commit choreography, not the feature). The M5 logic
    // is covered without it: `honored_revocations` (the origin cross-check; the security core) is
    // unit-tested above deterministically; the write/read of a revocation shares the owner-signed
    // `Devices`-doc path exercised by `a_forged_devices_document_entry_is_ignored`; and the MLS
    // Remove the owner performs is `ChannelSync::request_remove`, covered by the removal tests in
    // `catcoms-sync`. `drain_outbox` is fire-and-forget, so the enforcement call never blocks.
    #[tokio::test]
    #[ignore = "harness choreography flake; see the note above; the M5 logic is covered elsewhere"]
    async fn an_origin_revokes_its_own_companion_and_the_owner_removes_the_leaf() {
        // M5: the "lost phone" verb. The origin signs a revocation; the owner enforces the MLS
        // Remove. Here alice is both owner and origin (the common single-member case).
        let hub = Hub::new();
        let clock = ManualClock::new(T0);
        let alice_peer = PeerId::from_u64(1);
        let mut alice = founder_on(&hub, alice_peer, &clock, 1);
        alice.subscribe_control().await.unwrap();
        alice.open_channel(GENERAL).await.unwrap();
        alice.open_devices().await.unwrap();

        let (secrets, _) = begin_pairing(&mut ChaCha20Rng::seed_from_u64(9)).unwrap();
        let phone_id = secrets.device_id();
        let grant = grant_from(&alice, phone_id, "phone");
        let _phone = present(
            &hub,
            2,
            alice_peer,
            &mut alice,
            &clock,
            secrets.device().duplicate().unwrap(),
            &grant,
        )
        .await
        .expect("admitted");
        let phone_fp = fingerprint(&phone_id);
        assert_eq!(alice.member_count(), 2);
        assert_eq!(alice.devices().len(), 1);

        // Revoke it. Only the origin may; a stranger fingerprint or a non-origin caller is refused.
        assert!(
            alice.revoke_device("deadbeef").await.is_err(),
            "no such device"
        );
        alice
            .revoke_device(&phone_fp)
            .await
            .expect("the origin revokes its own device");

        // The owner enforces the removal on its next reconcile ticks. (Re-admission is separately
        // blocked by the spent-certificate ledger; see `one_certificate_admits_one_device_once`;
        // and the revoked set, without needing a fragile second wire round-trip here.)
        drain_sync(&mut alice).await;
        assert!(
            !alice.members().contains(&phone_id),
            "the revoked companion's leaf is removed"
        );
        assert_eq!(alice.member_count(), 1);
    }

    #[tokio::test]
    async fn one_origin_cannot_admit_unbounded_devices() {
        // Adversarial-review BLOCKING: without a per-origin cap, one member could drive the owner
        // to run unbounded MLS Adds (the ceremony's human gate is on the origin, i.e. the
        // attacker's own device). The owner caps how many companions one origin may have.
        let hub = Hub::new();
        let clock = ManualClock::new(T0);
        let alice_peer = PeerId::from_u64(1);
        let mut alice = founder_on(&hub, alice_peer, &clock, 1);
        alice.subscribe_control().await.unwrap();
        alice.open_channel(GENERAL).await.unwrap();
        alice.open_devices().await.unwrap();

        // Present far more devices than the cap, all certified by the same origin (alice). Count
        // how many the owner admits; the count must plateau and later presentations must fail.
        let mut admitted = 0usize;
        let mut saw_refusal = false;
        for i in 0..12u64 {
            let (secrets, _) = begin_pairing(&mut ChaCha20Rng::seed_from_u64(100 + i)).unwrap();
            let grant = grant_from(&alice, secrets.device_id(), "dev");
            let r = present(
                &hub,
                200 + i,
                alice_peer,
                &mut alice,
                &clock,
                secrets.device().duplicate().unwrap(),
                &grant,
            )
            .await;
            match r {
                Ok(_) => admitted += 1,
                Err(_) => saw_refusal = true,
            }
        }
        assert!(saw_refusal, "the owner must refuse past the per-origin cap");
        assert!(
            admitted < 12,
            "not every presented device may be admitted (got {admitted})"
        );
        // Every admitted companion is a real leaf; the count is 1 (alice) + the capped companions.
        assert_eq!(alice.member_count(), 1 + admitted);
        assert_eq!(alice.devices().len(), admitted);
    }

    #[tokio::test]
    async fn a_certificate_minted_for_another_server_cannot_admit_here() {
        // The M2 review's requirement, enforced cryptographically: `group_id` is inside the
        // origin's signature, so a certificate minted on server A is inert on server B.
        let hub = Hub::new();
        let clock = ManualClock::new(T0);
        let a_peer = PeerId::from_u64(1);
        let b_peer = PeerId::from_u64(2);
        let mut server_a = founder_on(&hub, a_peer, &clock, 1);
        let mut server_b = founder_on(&hub, b_peer, &clock, 2);
        server_a.subscribe_control().await.unwrap();
        server_b.subscribe_control().await.unwrap();
        assert_ne!(server_a.group_id(), server_b.group_id());

        let (secrets, _) = begin_pairing(&mut ChaCha20Rng::seed_from_u64(9)).unwrap();
        // A *valid* grant; for server A; presented to server B.
        let for_a = grant_from(&server_a, secrets.device_id(), "phone");
        let smuggled = PerServerGrant {
            group_id: server_b.group_id(),
            owner_public_key: server_b.owner_public_key().unwrap(),
            ..for_a
        };
        let err = present(
            &hub,
            3,
            b_peer,
            &mut server_b,
            &clock,
            secrets.device().duplicate().unwrap(),
            &smuggled,
        )
        .await
        .expect_err("a certificate for another group must not admit");
        assert!(matches!(err, AppError::Sync(_)), "rejected, not timed out");
        assert_eq!(server_b.member_count(), 1);
    }

    #[tokio::test]
    async fn a_certificate_from_a_non_member_origin_is_refused() {
        // The forgery a new device could always attempt: mint its own "origin" keypair and sign
        // itself a certificate. Nothing stops it *signing* one; it is inert because the signer
        // is not on the roster.
        let hub = Hub::new();
        let clock = ManualClock::new(T0);
        let alice_peer = PeerId::from_u64(1);
        let mut alice = founder_on(&hub, alice_peer, &clock, 1);
        alice.subscribe_control().await.unwrap();

        let (secrets, _) = begin_pairing(&mut ChaCha20Rng::seed_from_u64(9)).unwrap();
        let stranger = catcoms_crypto::DeviceKeypair::generate(&mut ChaCha20Rng::seed_from_u64(77));
        let forged = DeviceCertificate::issue(
            &stranger,
            secrets.device_id(),
            &alice.group_id(),
            "phone",
            T0,
        )
        .unwrap();
        // It is a perfectly well-formed statement…
        assert!(forged.verify(&forged.origin_id));
        // …and admits nothing.
        let grant = grant_with(&alice, forged);
        assert!(present(
            &hub,
            2,
            alice_peer,
            &mut alice,
            &clock,
            secrets.device().duplicate().unwrap(),
            &grant,
        )
        .await
        .is_err());
        assert_eq!(alice.member_count(), 1);

        // And there is no "I certify myself" statement to make in the first place.
        assert!(DeviceCertificate::issue(
            &stranger,
            stranger.device_id(),
            &alice.group_id(),
            "me",
            T0
        )
        .is_err());
    }

    #[tokio::test]
    async fn a_stale_certificate_is_refused() {
        // Certificates carry no expiry by design, so the admitting owner enforces `issued_ts`
        // freshness instead (design-multi-device.md v2.2).
        let hub = Hub::new();
        let clock = ManualClock::new(T0);
        let alice_peer = PeerId::from_u64(1);
        let mut alice = founder_on(&hub, alice_peer, &clock, 1);
        alice.subscribe_control().await.unwrap();

        let (secrets, _) = begin_pairing(&mut ChaCha20Rng::seed_from_u64(9)).unwrap();
        let grant = grant_from(&alice, secrets.device_id(), "phone");
        assert_eq!(grant.certificate.issued_ts_ms, T0);

        // Just inside the window: still admitted.
        clock.set_ms(T0 + CERT_AGE - 1);
        assert!(present(
            &hub,
            2,
            alice_peer,
            &mut alice,
            &clock,
            secrets.device().duplicate().unwrap(),
            &grant,
        )
        .await
        .is_ok());

        // A second, equally valid grant, presented one millisecond past the window.
        let (later, _) = begin_pairing(&mut ChaCha20Rng::seed_from_u64(10)).unwrap();
        let stale = grant_from(&alice, later.device_id(), "laptop");
        clock.set_ms(T0 + 2 * CERT_AGE + 1);
        assert!(present(
            &hub,
            3,
            alice_peer,
            &mut alice,
            &clock,
            later.device().duplicate().unwrap(),
            &stale,
        )
        .await
        .is_err());
        assert_eq!(alice.member_count(), 2, "only the fresh one landed");
    }

    #[tokio::test]
    async fn one_certificate_admits_one_device_once() {
        // Replay: a device removed from the group re-presents the certificate it was admitted
        // with. The bind nonce is consumed in the (persisted) invite ledger, so it is inert;
        // "already a member" is not the only thing standing in the way.
        let hub = Hub::new();
        let clock = ManualClock::new(T0);
        let alice_peer = PeerId::from_u64(1);
        let mut alice = founder_on(&hub, alice_peer, &clock, 1);
        alice.subscribe_control().await.unwrap();
        alice.open_devices().await.unwrap();

        let (secrets, _) = begin_pairing(&mut ChaCha20Rng::seed_from_u64(9)).unwrap();
        let grant = grant_from(&alice, secrets.device_id(), "phone");
        assert!(present(
            &hub,
            2,
            alice_peer,
            &mut alice,
            &clock,
            secrets.device().duplicate().unwrap(),
            &grant,
        )
        .await
        .is_ok());
        assert_eq!(alice.member_count(), 2);

        // The owner removes it…
        let phone_fp = fingerprint(&secrets.device_id());
        alice.remove_member(&phone_fp).await.unwrap();
        assert_eq!(alice.member_count(), 1);

        // …and the same grant, on a fresh KeyPackage, admits nothing a second time.
        assert!(present(
            &hub,
            3,
            alice_peer,
            &mut alice,
            &clock,
            secrets.device().duplicate().unwrap(),
            &grant,
        )
        .await
        .is_err());
        assert_eq!(alice.member_count(), 1);
    }

    #[tokio::test]
    async fn a_companion_may_not_certify_a_further_device() {
        // Chain depth is 1: a stolen companion must never be an identity factory. The owner
        // refuses because the certifier is in the `Devices` registry as somebody's companion.
        let hub = Hub::new();
        let clock = ManualClock::new(T0);
        let alice_peer = PeerId::from_u64(1);
        let mut alice = founder_on(&hub, alice_peer, &clock, 1);
        alice.subscribe_control().await.unwrap();
        alice.open_devices().await.unwrap();

        let (secrets, _) = begin_pairing(&mut ChaCha20Rng::seed_from_u64(9)).unwrap();
        let grant = grant_from(&alice, secrets.device_id(), "phone");
        let phone = present(
            &hub,
            2,
            alice_peer,
            &mut alice,
            &clock,
            secrets.device().duplicate().unwrap(),
            &grant,
        )
        .await
        .unwrap();
        assert!(alice
            .devices()
            .contains_key(&fingerprint(&secrets.device_id())));

        // The companion is a full member, so it *can* sign a certificate; its own key is on the
        // roster. The gate is that the owner knows it is a companion, not an origin.
        let (tablet, _) = begin_pairing(&mut ChaCha20Rng::seed_from_u64(11)).unwrap();
        let sub_grant = PerServerGrant {
            owner_public_key: alice.owner_public_key().unwrap(),
            certificate: phone
                .issue_device_certificate(tablet.device_id(), "tablet")
                .unwrap(),
            ..grant_from(&alice, tablet.device_id(), "tablet")
        };
        assert!(present(
            &hub,
            3,
            alice_peer,
            &mut alice,
            &clock,
            tablet.device().duplicate().unwrap(),
            &sub_grant,
        )
        .await
        .is_err());
        assert_eq!(alice.member_count(), 2, "the second-hop device stayed out");
    }

    #[tokio::test]
    async fn a_tampered_certificate_is_refused() {
        // Every field is inside the signature, so any edit invalidates it. Both ends apply the
        // same predicate (`DeviceCertificate::verify`), so an honest client refuses to present a
        // tampered certificate at all; and a modified one gets nowhere either.
        let hub = Hub::new();
        let clock = ManualClock::new(T0);
        let alice_peer = PeerId::from_u64(1);
        let mut alice = founder_on(&hub, alice_peer, &clock, 1);
        alice.subscribe_control().await.unwrap();

        let (secrets, _) = begin_pairing(&mut ChaCha20Rng::seed_from_u64(9)).unwrap();
        let good = grant_from(&alice, secrets.device_id(), "phone");

        for mutate in [0u8, 1, 2] {
            let mut cert = good.certificate.clone();
            match mutate {
                0 => cert.device_name = "ADMIN".into(),
                1 => cert.issued_ts_ms = T0 - 1,
                _ => cert.signature[0] ^= 0x01,
            }
            assert!(
                !cert.verify(&cert.origin_id),
                "tamper {mutate} must not verify"
            );
            let grant = grant_with(&alice, cert);
            assert!(present(
                &hub,
                2 + u64::from(mutate),
                alice_peer,
                &mut alice,
                &clock,
                secrets.device().duplicate().unwrap(),
                &grant,
            )
            .await
            .is_err());
        }
        assert_eq!(alice.member_count(), 1);
    }

    #[tokio::test]
    async fn a_grant_admitted_for_one_device_cannot_be_completed_on_another() {
        let hub = Hub::new();
        let clock = ManualClock::new(T0);
        let alice_peer = PeerId::from_u64(1);
        let mut alice = founder_on(&hub, alice_peer, &clock, 1);
        alice.subscribe_control().await.unwrap();

        let (mine, _) = begin_pairing(&mut ChaCha20Rng::seed_from_u64(9)).unwrap();
        let (other, _) = begin_pairing(&mut ChaCha20Rng::seed_from_u64(10)).unwrap();
        let grant = grant_from(&alice, mine.device_id(), "phone");

        // The nearest thing to a redirect an honest client can even attempt: another device
        // presenting this grant. Refused before a byte leaves; the certificate names one key.
        assert!(present(
            &hub,
            2,
            alice_peer,
            &mut alice,
            &clock,
            other.device().duplicate().unwrap(),
            &grant,
        )
        .await
        .is_err());
        assert_eq!(alice.member_count(), 1);

        // And the reason a *modified* client (or a relay that steals the Welcome off the wire)
        // gains nothing either: MLS seals a Welcome's group secrets to the init key of the
        // KeyPackage it was built for, and that private key exists only in that device's
        // provider. The device keypair itself is what binds a Welcome to its device.
        let owner = MlsDevice::generate().unwrap();
        let mut group = ServerGroup::create(&owner).unwrap();
        let x = MlsDevice::generate().unwrap();
        let y = MlsDevice::generate().unwrap();
        let outcome = group.add_member(&owner, x.key_package().unwrap()).unwrap();
        assert!(
            ServerGroup::join(&y, &outcome.welcome).is_err(),
            "a Welcome minted for X is inert on Y"
        );
        assert!(ServerGroup::join(&x, &outcome.welcome).is_ok());
    }

    // Write an entry the way the owner does: certificate + the owner's signature over it.
    fn owner_sign_entry(
        doc: &mut AutoCommit,
        cert: &DeviceCertificate,
        gid: &[u8],
        owner: &catcoms_crypto::DeviceKeypair,
    ) {
        let fp = fingerprint(&cert.new_device_id);
        let payload = device_entry_payload(gid, &fp, &cert.encode());
        let sig = owner.sign(&payload);
        let vk = owner.verifying_key();
        write_device_entry(doc, cert, vk.as_bytes(), &sig).unwrap();
    }

    #[test]
    fn a_forged_devices_document_entry_is_ignored() {
        // The registry is a CRDT any member can write into. An entry is trusted only if the
        // CURRENT OWNER signed it; a device certificate alone proves an origin *wanted* a device,
        // never that the group *admitted* it, and its subject can be any device id.
        let owner = catcoms_crypto::DeviceKeypair::generate(&mut ChaCha20Rng::seed_from_u64(9));
        let owner_id = owner.device_id();
        let alice = catcoms_crypto::DeviceKeypair::generate(&mut ChaCha20Rng::seed_from_u64(1));
        let victim = catcoms_crypto::DeviceKeypair::generate(&mut ChaCha20Rng::seed_from_u64(2));
        let gid = vec![7u8; 8];
        let real = DeviceCertificate::issue(&alice, victim.device_id(), &gid, "phone", T0).unwrap();

        // An owner-signed entry reads.
        let mut doc = AutoCommit::new();
        owner_sign_entry(&mut doc, &real, &gid, &owner);
        assert_eq!(
            read_device_certs(&doc, &gid, &owner_id).len(),
            1,
            "a genuine entry reads"
        );
        // Group-bound: the same doc read against another group is empty.
        assert!(read_device_certs(&doc, b"other-group", &owner_id).is_empty());
        // Owner-bound: a DIFFERENT owner id rejects it (a new owner cannot inherit stale entries).
        let other_owner =
            catcoms_crypto::DeviceKeypair::generate(&mut ChaCha20Rng::seed_from_u64(8));
        assert!(read_device_certs(&doc, &gid, &other_owner.device_id()).is_empty());

        // THE FORGERY (BLOCKING 1): a member writes its own genuinely-signed certificate directly,
        // with NO owner signature. Alice certifies the victim's own origin id as a "companion";
        // a real signature by a real member; and posts it. It must NOT read, or it would poison
        // the depth-1 gate and spoof the victim's attribution.
        let forged =
            DeviceCertificate::issue(&alice, victim.device_id(), &gid, "phone", T0).unwrap();
        let mut poisoned = AutoCommit::new();
        write_device_entry(&mut poisoned, &forged, &[0u8; 32], &[0u8; 64]).unwrap();
        assert!(
            read_device_certs(&poisoned, &gid, &owner_id).is_empty(),
            "an entry the owner never signed must be ignored"
        );

        // An entry signed by a member pretending to be the owner (their own key) is rejected;
        // the signing key must content-address the owner id.
        let mut impostor = AutoCommit::new();
        owner_sign_entry(&mut impostor, &real, &gid, &alice);
        assert!(read_device_certs(&impostor, &gid, &owner_id).is_empty());

        // Sibling-field tamper and junk-cert entries still drop even when owner-signed over the
        // ORIGINAL cert (the signature is over the stored cert bytes, so mutating a sibling breaks
        // the field cross-check; mutating the cert bytes breaks the owner signature).
        let mut lying = AutoCommit::new();
        owner_sign_entry(&mut lying, &real, &gid, &owner);
        let devices = match lying.get(ROOT, D_DEVICES).unwrap() {
            Some((Value::Object(ObjType::Map), id)) => id,
            _ => unreachable!(),
        };
        let entry = match lying
            .get(&devices, fingerprint(&victim.device_id()))
            .unwrap()
        {
            Some((Value::Object(ObjType::Map), id)) => id,
            _ => unreachable!(),
        };
        lying.put(&entry, D_NAME, "ADMIN").unwrap();
        assert!(read_device_certs(&lying, &gid, &owner_id).is_empty());

        // The revocation slot is empty until M5 writes it.
        assert!(read_device_revocations(&doc).is_empty());
    }

    #[test]
    fn channel_id_is_deterministic_normalized_and_distinct() {
        // Same name (modulo case/whitespace) → same id, so members converge.
        assert_eq!(channel_id("general"), channel_id(" General "));
        assert_eq!(channel_id("Random"), channel_id("random"));
        // Distinct names → distinct ids.
        assert_ne!(channel_id("general"), channel_id("random"));
    }

    #[tokio::test]
    async fn found_open_send_and_read_a_channel() {
        let mut alice = founder();
        alice.open_channel(GENERAL).await.unwrap();
        assert!(alice.messages(GENERAL).is_empty());
        alice.send_message(GENERAL, "hello world").await.unwrap();

        let msgs = alice.messages(GENERAL);
        assert_eq!(msgs.len(), 1);
        // Messages are authored by device fingerprint; the name resolves from the profile.
        assert_eq!(msgs[0].author, alice.my_fingerprint());
        assert_eq!(msgs[0].text, "hello world");
        assert_eq!(msgs[0].ts, 1_000, "stamped from the injected clock");
        assert_eq!(alice.member_count(), 1);
        assert_eq!(alice.display_name(), "alice");
        let roster = alice.members_view();
        assert_eq!(roster.len(), 1);
        assert!(roster[0].is_self, "the founder sees itself in the roster");
        assert_eq!(roster[0].fingerprint.len(), 8, "4-byte hex fingerprint");
    }

    #[tokio::test]
    async fn a_channel_topic_round_trips_and_is_size_capped() {
        let mut alice = founder();
        alice.open_channel(GENERAL).await.unwrap();
        assert_eq!(alice.channel_topic(GENERAL), "", "no topic by default");

        alice
            .set_channel_topic(GENERAL, "shipping the thing")
            .await
            .unwrap();
        assert_eq!(alice.channel_topic(GENERAL), "shipping the thing");
        // Setting it again is an idempotent no-op, and the messages are untouched by a topic op.
        alice
            .set_channel_topic(GENERAL, "shipping the thing")
            .await
            .unwrap();
        assert!(alice.messages(GENERAL).is_empty());

        // A topic is replaced (last-writer-wins) and can be cleared.
        alice.set_channel_topic(GENERAL, "shipped").await.unwrap();
        assert_eq!(alice.channel_topic(GENERAL), "shipped");
        alice.set_channel_topic(GENERAL, "").await.unwrap();
        assert_eq!(alice.channel_topic(GENERAL), "");

        // Exactly at the cap is fine; one byte over is refused (and changes nothing).
        let at_cap = "t".repeat(MAX_CHANNEL_TOPIC_BYTES);
        alice.set_channel_topic(GENERAL, &at_cap).await.unwrap();
        assert_eq!(alice.channel_topic(GENERAL), at_cap);
        let over = "t".repeat(MAX_CHANNEL_TOPIC_BYTES + 1);
        assert!(alice.set_channel_topic(GENERAL, &over).await.is_err());
        assert_eq!(alice.channel_topic(GENERAL), at_cap, "unchanged");
        // The cap counts UTF-8 *bytes*, not chars; a multi-byte topic over budget is refused too.
        let multibyte = "é".repeat(MAX_CHANNEL_TOPIC_BYTES / 2 + 1); // 2 bytes each
        assert!(alice.set_channel_topic(GENERAL, &multibyte).await.is_err());
    }

    #[tokio::test]
    async fn a_jukebox_queue_validates_its_entries_and_is_count_capped() {
        let mut alice = founder();
        alice.open_channel(GENERAL).await.unwrap();
        assert!(alice.jukebox(GENERAL).is_empty(), "no queue by default");

        let id = alice
            .jukebox_add(GENERAL, "deadbeef", "Track One")
            .await
            .unwrap();
        let queue = alice.jukebox(GENERAL);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].id, id);
        assert_eq!(queue[0].cid, "deadbeef");
        assert_eq!(queue[0].name, "Track One");
        assert_eq!(queue[0].author, alice.my_fingerprint());
        assert_eq!(queue[0].added_ms, 1_000, "stamped from the injected clock");
        // A queue op is not a message; the channel's chat is untouched.
        assert!(alice.messages(GENERAL).is_empty());

        // A removal addresses exactly one entry, and removing a missing entry is a no-op.
        alice.jukebox_remove(GENERAL, &id).await.unwrap();
        assert!(alice.jukebox(GENERAL).is_empty());
        alice.jukebox_remove(GENERAL, &id).await.unwrap();
        alice.jukebox_remove(GENERAL, "nosuchentry").await.unwrap();

        // A cid must be lowercase hex within budget, and a name must be non-blank and within
        // its byte budget; every refusal leaves the queue as it was.
        for bad in ["", "DEADBEEF", "dead beef", "deadbeefg", "0x12"] {
            assert!(
                alice.jukebox_add(GENERAL, bad, "Track").await.is_err(),
                "cid {bad:?} is not a content address"
            );
        }
        let long_cid = "a".repeat(MAX_JUKEBOX_CID_CHARS + 1);
        assert!(alice
            .jukebox_add(GENERAL, &long_cid, "Track")
            .await
            .is_err());
        assert!(alice.jukebox_add(GENERAL, "beef", "   ").await.is_err());
        let over = "n".repeat(MAX_JUKEBOX_NAME_BYTES + 1);
        assert!(alice.jukebox_add(GENERAL, "beef", &over).await.is_err());
        // The name cap counts UTF-8 *bytes*, not chars, like the topic cap.
        let multibyte = "é".repeat(MAX_JUKEBOX_NAME_BYTES / 2 + 1); // 2 bytes each
        assert!(alice
            .jukebox_add(GENERAL, "beef", &multibyte)
            .await
            .is_err());
        assert!(alice.jukebox(GENERAL).is_empty(), "nothing was queued");

        // Exactly at the cap is fine; the next add is refused and changes nothing.
        for i in 0..MAX_JUKEBOX_ENTRIES {
            alice
                .jukebox_add(GENERAL, &format!("bee{i:x}"), &format!("Track {i}"))
                .await
                .unwrap();
        }
        assert_eq!(alice.jukebox(GENERAL).len(), MAX_JUKEBOX_ENTRIES);
        assert!(alice
            .jukebox_add(GENERAL, "beef", "One Too Many")
            .await
            .is_err());
        assert_eq!(alice.jukebox(GENERAL).len(), MAX_JUKEBOX_ENTRIES);
    }

    // ---------- steady-state peer exchange, at the product layer (defect P1) ----------
    //
    // The PEX machinery below has had passing unit tests in `catcoms-sync` since 6e-3d-7 while
    // being completely unwired in the product: nothing outside those tests ever called
    // `publish_self_record`, `request_pex` or `known_peer_records`. `peer_records` was therefore
    // permanently empty in the shipping app, which silently took three shipped features with it:
    // the roster's online dots always read zero, the Phase 9g cross-session re-dial had nothing
    // to re-dial, and the eclipse detector computed `reachable = 1` and raised CAUTION
    // unconditionally for every group of four or more.
    //
    // These tests live HERE, at the `catcoms-app` layer, precisely because the sync-layer tests
    // cannot catch that class of bug: they call the primitives directly, so they pass whether or
    // not anything above them does. This drives the same entry points the actor's discovery tick
    // drives, and asserts on the product-facing answer (`online_members`).

    #[test]
    fn an_unclaimed_infrastructure_connection_is_not_a_fetchable_member() {
        let hub = Hub::new();
        let mut server = Server::found(
            hub.join(PeerId::from_u64(1)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(99),
            Box::new(ManualClock::new(1_000)),
            "alice",
        )
        .unwrap();
        let relay = PeerId::from_u64(9_999);
        let mut handoff = PreOwnerConnectionHandoff::default();
        assert!(handoff.observe(&catcoms_rt::TransportEvent::PeerConnected(relay)));
        server.sync.adopt_pre_owner_connections(handoff);

        assert!(
            server.sync.has_connected_peer(),
            "the transport fact exists for relay/rendezvous bookkeeping"
        );
        assert!(server.online_members().is_empty());
        assert!(
            !server.has_fetch_peers(),
            "an unclaimed infrastructure socket cannot be labelled as a member/blob source"
        );
    }

    #[tokio::test]
    async fn two_members_exchange_records_and_report_each_other_online() {
        let hub = Hub::new();
        let alice_peer = test_transport_peer(1);
        let bob_peer = test_transport_peer(2);
        let mut alice = Server::found(
            hub.join(alice_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
            "alice",
        )
        .unwrap();
        alice.subscribe_control().await.unwrap();

        let invite = alice.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (bob, _) = tokio::join!(
            Server::join(
                hub.join(bob_peer),
                MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(2),
                Box::new(ManualClock::new(1_000)),
                "bob",
                alice_peer,
                &invite,
            ),
            alice.sync_once(),
        );
        let mut bob = bob.unwrap();
        assert_eq!(bob.member_count(), 2);

        // Before anything publishes, this is exactly the shipping app's state: a two-member group
        // that has been talking, and a roster with every dot dark.
        assert!(
            alice.online_members().is_empty() && bob.online_members().is_empty(),
            "the bug being regressed: records are empty, so nobody is ever online"
        );

        // What found/join/reload now do: publish a signed record on this launch's reserved
        // sequence block. (Addresses are the reachable ones; loopback and LAN entries are
        // stripped at publish, so a stand-in public address is what a real node would carry.)
        alice
            .publish_self_record(
                vec![test_peer_route(1, "/ip4/203.0.113.1/tcp/9000")],
                65_536,
            )
            .unwrap();
        bob.publish_self_record(
            vec![test_peer_route(2, "/ip4/203.0.113.2/tcp/9000")],
            65_536,
        )
        .unwrap();

        // What the actor's discovery tick now does: one PEX pass each, nobody naming a peer.
        let (_, _) = tokio::join!(bob.drive_pex(), alice.sync_once());
        let (_, _) = tokio::join!(alice.drive_pex(), bob.sync_once());

        assert_eq!(
            alice.online_members(),
            vec![bob.my_fingerprint()],
            "Alice sees Bob online"
        );
        assert_eq!(
            bob.online_members(),
            vec![alice.my_fingerprint()],
            "…and Bob sees Alice"
        );

        // The delivery-state "reachable" count reads off the same signal, so it moves too.
        assert_eq!(alice.online_members().len(), 1);

        // Phase 9g: the records land in the snapshot, so a reloading node has somewhere to dial.
        // This returned an empty list for the entire life of the feature.
        let snap = alice.snapshot().unwrap();
        let addrs = peer_addrs_from_snapshot(&snap).unwrap();
        let bob_route = test_peer_route(2, "/ip4/203.0.113.2/tcp/9000");
        assert!(
            addrs.contains(&bob_route),
            "the cross-session re-dial has Bob's address to dial, got {addrs:?}"
        );

        // And the cross-session cache picks up the proven member for the next launch.
        assert_eq!(alice.cache_known_records(), 1);
        let key = [9u8; 32];
        let bytes = alice.address_cache_bytes(&key);
        assert!(bob.load_address_cache(&bytes, &key));
        assert_eq!(bob.cached_peer_count(), 1);
    }

    #[tokio::test]
    async fn the_eclipse_advisory_stays_quiet_for_a_healthy_group() {
        // The third consequence of P1: with `reachable_devices` stuck at 1, the suspect predicate
        // was unconditionally true for any roster above the floor, so CAUTION fired about 30s
        // after startup, forever, in every real group. Four members, all reachable, must be quiet.
        let hub = Hub::new();
        let alice_peer = test_transport_peer(1);
        let clock = ManualClock::new(1_000);
        let mut alice = Server::found(
            hub.join(alice_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(clock.clone()),
            "alice",
        )
        .unwrap();
        alice.subscribe_control().await.unwrap();
        alice
            .publish_self_record(
                vec![test_peer_route(1, "/ip4/203.0.113.1/tcp/9000")],
                65_536,
            )
            .unwrap();

        let mut members = Vec::new();
        for (n, nonce) in [(2u8, 11u8), (3, 12), (4, 13)] {
            let invite = alice.mint_invite([nonce; 16], u64::MAX, vec![]).unwrap();
            let (joined, _) = tokio::join!(
                Server::join(
                    hub.join(test_transport_peer(n)),
                    MlsDevice::generate().unwrap(),
                    ChaCha20Rng::seed_from_u64(u64::from(n)),
                    Box::new(clock.clone()),
                    "member",
                    alice_peer,
                    &invite,
                ),
                alice.sync_once(),
            );
            let mut m = joined.unwrap();
            m.publish_self_record(
                vec![test_peer_route(n, &format!("/ip4/203.0.113.{n}/tcp/9000"))],
                65_536,
            )
            .unwrap();
            members.push(m);
        }
        assert_eq!(alice.member_count(), 4);
        let mut it = members.into_iter();
        let mut m2 = it.next().unwrap();
        let mut m3 = it.next().unwrap();
        let mut m4 = it.next().unwrap();

        // One PEX pass: Alice asks each of them in turn (that is what the tick does; it names no
        // peer), and all three serve concurrently, as three separate processes would.
        let (_, _, _, _) = tokio::join!(
            alice.drive_pex(),
            m2.sync_once(),
            m3.sync_once(),
            m4.sync_once(),
        );
        assert_eq!(
            alice.online_members().len(),
            3,
            "every other member is reachable"
        );

        // Well past the detector's 30s grace window, and still quiet.
        for _ in 0..5 {
            assert!(!alice.observe_eclipse(), "a healthy group must not warn");
            clock.advance_ms(60_000);
        }
        assert!(!alice.observe_eclipse());
    }

    #[tokio::test]
    async fn a_jukebox_queue_round_trips_between_two_members() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice = Server::found(
            hub.join(alice_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
            "alice",
        )
        .unwrap();
        alice.subscribe_control().await.unwrap();
        alice.open_channel(GENERAL).await.unwrap();
        let id = alice
            .jukebox_add(GENERAL, "cafe01", "Opening Theme")
            .await
            .unwrap();

        // Bob joins via an invite, over the hub (Alice serves the join with a tick).
        let invite = alice.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let bob_net = hub.join(PeerId::from_u64(2));
        let (bob, _) = tokio::join!(
            Server::join(
                bob_net,
                MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(2),
                Box::new(ManualClock::new(2_000)),
                "bob",
                alice_peer,
                &invite,
            ),
            alice.sync_once(),
        );
        let mut bob = bob.unwrap();

        // The queue rides the channel document, so catch-up delivers it with the messages.
        bob.open_channel(GENERAL).await.unwrap();
        let (applied, _) = tokio::join!(
            bob.request_channel_catchup(alice_peer, GENERAL),
            alice.sync_once(),
        );
        assert_eq!(applied.unwrap(), 1, "Bob applied Alice's queue op");
        let queue = bob.jukebox(GENERAL);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].id, id);
        assert_eq!(queue[0].cid, "cafe01");
        assert_eq!(queue[0].name, "Opening Theme");
        assert_eq!(
            queue[0].author,
            alice.my_fingerprint(),
            "the entry carries Alice's authorship, not the reader's"
        );

        // …and *any* member may prune it; Bob is not the owner and is not refused.
        assert_ne!(bob.my_role(), Role::Owner);
        bob.jukebox_remove(GENERAL, &id).await.unwrap();
        assert!(bob.jukebox(GENERAL).is_empty());
        assert!(alice.sync_once().await.unwrap());
        assert!(
            alice.jukebox(GENERAL).is_empty(),
            "the removal reached Alice"
        );
    }

    #[tokio::test]
    async fn a_member_edits_and_deletes_its_own_messages_by_stable_id() {
        let mut alice = founder();
        alice.open_channel(GENERAL).await.unwrap();
        alice.send_message(GENERAL, "first").await.unwrap();
        alice.send_message(GENERAL, "secnod").await.unwrap();

        let msgs = alice.messages(GENERAL);
        assert_eq!(msgs.len(), 2);
        let id0 = msgs[0].id.clone();
        let id1 = msgs[1].id.clone();
        assert!(!id1.is_empty(), "new messages get a stable id");
        assert_ne!(id0, id1, "ids are unique");
        assert_eq!(msgs[1].edited, 0, "an unedited message has no edit stamp");

        // Fix the typo in the second message.
        alice.edit_message(GENERAL, &id1, "second").await.unwrap();
        let msgs = alice.messages(GENERAL);
        assert_eq!(msgs.len(), 2, "editing doesn't change the count");
        assert_eq!(msgs[1].text, "second");
        assert!(msgs[1].edited > 0, "edited is stamped");
        assert_eq!(msgs[1].id, id1, "the id is stable across an edit");

        // Delete the first message.
        alice.delete_message(GENERAL, &id0).await.unwrap();
        let msgs = alice.messages(GENERAL);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "second", "the right message remains");
        assert_eq!(msgs[0].id, id1);

        // Editing/deleting an unknown (or not-your-own) message is refused.
        assert!(alice.edit_message(GENERAL, "deadbeef", "x").await.is_err());
        assert!(alice.delete_message(GENERAL, "deadbeef").await.is_err());
    }

    #[tokio::test]
    async fn an_owner_can_moderation_delete_another_members_message() {
        let mut alice = founder();
        alice.open_channel(GENERAL).await.unwrap();
        assert_eq!(alice.my_role(), Role::Owner);

        // Inject a message authored by someone else (as if it had arrived over gossip).
        alice
            .sync
            .post(DocType::Channel, GENERAL, move |d| {
                append_message(d, "mid-01", "beefbeef", "spam", 5, "").map(|_| ())
            })
            .await
            .unwrap();
        assert!(alice
            .messages(GENERAL)
            .iter()
            .any(|m| m.author == "beefbeef"));

        // An owner may delete it even though it isn't theirs (moderation); editing it is still
        // refused (moderation is delete-only; no impersonation).
        assert!(alice.edit_message(GENERAL, "mid-01", "x").await.is_err());
        alice.delete_message(GENERAL, "mid-01").await.unwrap();
        assert!(!alice.messages(GENERAL).iter().any(|m| m.id == "mid-01"));
    }

    /// A channel that has carried messages since before ids existed holds several rows whose id is
    /// `""`, and matching on that field cannot tell them apart. `edit_message` is where that stops
    /// being untidy and becomes a rewrite: the author check runs over the caller's own messages and
    /// the write runs over everybody's, so an empty id passes the first on one message and lands
    /// the second on another. Found by adversarial review.
    #[tokio::test]
    async fn a_channel_command_with_no_message_id_is_refused_rather_than_aimed_at_random() {
        let mut alice = founder();
        alice.open_channel(GENERAL).await.unwrap();
        let mine = alice.my_fingerprint();

        // Two id-less messages, somebody else's first, injected as if they had arrived over gossip
        // from a client that predated ids.
        alice
            .sync
            .post(DocType::Channel, GENERAL, move |d| {
                append_message(d, "", "beefbeef", "theirs", 5, "")
            })
            .await
            .unwrap();
        alice
            .sync
            .post(DocType::Channel, GENERAL, move |d| {
                append_message(d, "", &mine, "mine", 6, "")
            })
            .await
            .unwrap();

        assert!(alice.edit_message(GENERAL, "", "mine now").await.is_err());
        assert!(alice.delete_message(GENERAL, "").await.is_err());
        assert!(alice.set_pin(GENERAL, "", true).await.is_err());
        assert!(alice.toggle_reaction(GENERAL, "", "👍").await.is_err());

        // Both messages are exactly as they were: the same words under the same fingerprints, and
        // the one that would have been hit is the one the caller never wrote.
        let msgs = alice.messages(GENERAL);
        assert_eq!(msgs.len(), 2, "nothing was deleted");
        assert_eq!(msgs[0].text, "theirs");
        assert_eq!(msgs[0].author, "beefbeef");
        assert_eq!(msgs[0].edited, 0, "and nobody's words were replaced");
        assert!(!msgs[0].pinned);
        assert!(msgs[0].reactions.is_empty());
        assert_eq!(msgs[1].text, "mine");
    }

    #[tokio::test]
    async fn signed_warning_evidence_and_advisory_kick_vote_converge() {
        let clock = ManualClock::new(T0);
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice = founder_on(&hub, alice_peer, &clock, 1);
        alice.subscribe_control().await.unwrap();
        alice.open_channel(GENERAL).await.unwrap();
        alice.open_moderation().await.unwrap();
        let invite = alice.mint_invite([8u8; 16], u64::MAX, vec![]).unwrap();
        let (bob, _) = tokio::join!(
            Server::join(
                hub.join(PeerId::from_u64(2)),
                MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(2),
                Box::new(clock.clone()),
                "bob",
                alice_peer,
                &invite,
            ),
            alice.sync_once(),
        );
        let mut bob = bob.unwrap();
        bob.open_channel(GENERAL).await.unwrap();
        bob.open_moderation().await.unwrap();

        bob.send_message(GENERAL, "a message retained as evidence")
            .await
            .unwrap();
        alice.sync_once().await.unwrap();
        let message = alice.messages(GENERAL).pop().unwrap();
        assert_eq!(message.author, bob.my_fingerprint());
        assert!(bob
            .warn_message(GENERAL, &message.id, "members cannot warn")
            .await
            .is_err());

        let warning_id = alice
            .warn_message(GENERAL, &message.id, "breaks the posted rules")
            .await
            .unwrap();
        bob.sync_once().await.unwrap();
        let warning = bob
            .moderation_state()
            .events
            .into_iter()
            .find(|event| event.id == warning_id)
            .unwrap();
        assert!(warning.signature_valid && warning.authorized);
        assert_eq!(warning.message_text, "a message retained as evidence");
        assert_eq!(warning.target, bob.my_fingerprint());

        let case_id = alice
            .create_kick_case(
                &bob.my_fingerprint(),
                "repeated disruption",
                std::slice::from_ref(&warning_id),
            )
            .await
            .unwrap();
        bob.sync_once().await.unwrap();
        bob.cast_kick_vote(&case_id, true).await.unwrap();
        alice.sync_once().await.unwrap();
        let state = alice.moderation_state();
        assert!(state.votes.iter().any(|vote| {
            vote.case_id == case_id
                && vote.voter == bob.my_fingerprint()
                && vote.yes
                && vote.signature_valid
        }));
        assert_eq!(alice.member_count(), 2, "a yes vote cannot remove a member");

        alice.resolve_kick_case(&case_id, false).await.unwrap();
        bob.sync_once().await.unwrap();
        assert!(bob.moderation_state().events.iter().any(|event| {
            event.kind == "case_resolution"
                && event.case_id == case_id
                && event.outcome == "dismissed"
                && event.signature_valid
                && event.authorized
        }));
        assert_eq!(alice.member_count(), 2, "dismissal preserves membership");
    }

    #[tokio::test]
    async fn an_owner_pins_and_unpins_a_message() {
        let mut alice = founder();
        alice.open_channel(GENERAL).await.unwrap();
        assert_eq!(alice.my_role(), Role::Owner);
        alice.send_message(GENERAL, "important").await.unwrap();
        let id = alice.messages(GENERAL)[0].id.clone();
        assert!(!alice.messages(GENERAL)[0].pinned);

        alice.set_pin(GENERAL, &id, true).await.unwrap();
        assert!(alice.messages(GENERAL)[0].pinned, "now pinned");

        // Pinning an already-pinned message is an idempotent no-op (Ok, no redundant op).
        alice.set_pin(GENERAL, &id, true).await.unwrap();
        assert!(alice.messages(GENERAL)[0].pinned);

        alice.set_pin(GENERAL, &id, false).await.unwrap();
        assert!(!alice.messages(GENERAL)[0].pinned, "unpinned");

        // Unknown message errors.
        assert!(alice.set_pin(GENERAL, "deadbeef", true).await.is_err());
    }

    #[tokio::test]
    async fn members_toggle_emoji_reactions_on_a_message() {
        let mut alice = founder();
        alice.open_channel(GENERAL).await.unwrap();
        alice.send_message(GENERAL, "ship it").await.unwrap();
        let id = alice.messages(GENERAL)[0].id.clone();
        let me = alice.my_fingerprint();
        assert!(alice.messages(GENERAL)[0].reactions.is_empty());

        // Add 👍; one reaction, by me.
        alice.toggle_reaction(GENERAL, &id, "👍").await.unwrap();
        let r = alice.messages(GENERAL)[0].reactions.clone();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].emoji, "👍");
        assert_eq!(r[0].by, vec![me.clone()]);

        // A second, distinct emoji.
        alice.toggle_reaction(GENERAL, &id, "❤").await.unwrap();
        assert_eq!(alice.messages(GENERAL)[0].reactions.len(), 2);

        // Toggling 👍 off drops it entirely (empty reactor set removed); ❤ remains.
        alice.toggle_reaction(GENERAL, &id, "👍").await.unwrap();
        let r = alice.messages(GENERAL)[0].reactions.clone();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].emoji, "❤");

        // Reacting to an unknown message errors.
        assert!(alice
            .toggle_reaction(GENERAL, "deadbeef", "👍")
            .await
            .is_err());
        // The flat-key invariant is enforced: a bad emoji (empty / contains the separator) is refused.
        let id = alice.messages(GENERAL)[0].id.clone();
        assert!(alice.toggle_reaction(GENERAL, &id, "").await.is_err());
        assert!(alice
            .toggle_reaction(GENERAL, &id, "a\u{1f}b")
            .await
            .is_err());
    }

    #[test]
    fn reactions_converge_under_concurrent_same_emoji() {
        // The core invariant of the flat-key design: two members reacting with the SAME emoji to the
        // SAME message while partitioned must BOTH survive the merge (no dropped reaction), and both
        // replicas must converge to identical state. Exercised at the CRDT layer via fork/merge.
        let mut base = AutoCommit::new();
        append_message(&mut base, "m1", "alice", "hi", 1, "").unwrap();
        let mut a = base.fork();
        let mut b = base.fork();

        // Concurrently: a reacts 👍 + 🎉; b reacts 👍 (same emoji as a, different reactor).
        toggle_reaction_in_doc(&mut a, "m1", "👍", "alice").unwrap();
        toggle_reaction_in_doc(&mut a, "m1", "🎉", "alice").unwrap();
        toggle_reaction_in_doc(&mut b, "m1", "👍", "bob").unwrap();

        a.merge(&mut b).unwrap();
        b.merge(&mut a).unwrap();

        let ra = read_messages(&a)[0].reactions.clone();
        let rb = read_messages(&b)[0].reactions.clone();
        assert_eq!(ra, rb, "the two replicas converge to identical reactions");

        // 👍 kept BOTH reactors; the concurrent same-emoji adds did not clobber each other.
        let thumbs = ra.iter().find(|r| r.emoji == "👍").expect("👍 present");
        let mut by = thumbs.by.clone();
        by.sort();
        assert_eq!(by, vec!["alice".to_string(), "bob".to_string()]);
        // 🎉 (alice only) survived the merge too.
        let party = ra.iter().find(|r| r.emoji == "🎉").expect("🎉 present");
        assert_eq!(party.by, vec!["alice".to_string()]);

        // Concurrent remove (alice un-👍s) vs add (bob 🔥s) from the merged state stays convergent.
        let mut c = a.fork();
        let mut d = a.fork();
        toggle_reaction_in_doc(&mut c, "m1", "👍", "alice").unwrap();
        toggle_reaction_in_doc(&mut d, "m1", "🔥", "bob").unwrap();
        c.merge(&mut d).unwrap();
        d.merge(&mut c).unwrap();
        assert_eq!(
            read_messages(&c)[0].reactions,
            read_messages(&d)[0].reactions,
            "remove/add across replicas still converges"
        );
        // alice's 👍 is gone (bob's 👍 remains), bob's 🔥 landed.
        let thumbs = read_messages(&c)[0]
            .reactions
            .iter()
            .find(|r| r.emoji == "👍")
            .expect("👍 still present (bob)")
            .by
            .clone();
        assert_eq!(thumbs, vec!["bob".to_string()]);
        assert!(read_messages(&c)[0]
            .reactions
            .iter()
            .any(|r| r.emoji == "🔥" && r.by == vec!["bob".to_string()]));
    }

    #[tokio::test]
    async fn a_reply_carries_its_parent_id() {
        let mut alice = founder();
        alice.open_channel(GENERAL).await.unwrap();
        alice.send_message(GENERAL, "parent").await.unwrap();
        let parent_id = alice.messages(GENERAL)[0].id.clone();
        assert!(!parent_id.is_empty());

        alice
            .send_reply(GENERAL, "child", &parent_id)
            .await
            .unwrap();
        let msgs = alice.messages(GENERAL);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].reply_to, "", "a plain message carries no parent");
        assert_eq!(
            msgs[1].reply_to, parent_id,
            "the reply points at its parent"
        );
        assert_eq!(msgs[1].text, "child");
    }

    #[tokio::test]
    async fn inbox_collects_mentions_and_replies_to_me() {
        let mut alice = founder();
        alice.open_channel(GENERAL).await.unwrap();
        alice.open_profiles().await.unwrap();
        alice
            .set_profile(Profile {
                name: "Alice".into(),
                color: String::new(),
                font: String::new(),
                effect: String::new(),
                description: String::new(),
                bubble: String::new(),
                avatar: Vec::new(),
                banner: Vec::new(),
            })
            .await
            .unwrap();

        // alice posts a message a reply can target.
        alice.send_message(GENERAL, "hi all").await.unwrap();
        let alice_id = alice.messages(GENERAL)[0].id.clone();

        // Inject a foreign message mentioning @[Alice] and a foreign reply to alice's message.
        let parent = alice_id.clone();
        alice
            .sync
            .post(DocType::Channel, GENERAL, move |d| {
                append_message(d, "m-mention", "beefbeef", "look here @[Alice] !", 10, "")?;
                append_message(d, "m-reply", "feedface", "agreed", 11, &parent)?;
                Ok(())
            })
            .await
            .unwrap();

        let inbox = alice.inbox(50);
        assert_eq!(
            inbox.len(),
            2,
            "the mention + the reply, not alice's own message"
        );
        assert_eq!(inbox[0].message_id, "m-reply", "newest first");

        let mention = inbox.iter().find(|i| i.message_id == "m-mention").unwrap();
        assert!(mention.mention && !mention.reply);
        assert_eq!(mention.author, "beefbeef");

        let reply = inbox.iter().find(|i| i.message_id == "m-reply").unwrap();
        assert!(reply.reply && !reply.mention);

        // My own messages are never in my inbox.
        let me = alice.my_fingerprint();
        assert!(!inbox.iter().any(|i| i.author == me));
    }

    #[test]
    fn mention_name_normalization_matches_the_marker_form() {
        // Must mirror the desktop `mentionName`, or inbox detection misses real mentions.
        assert_eq!(normalize_mention_name("Alice"), "Alice");
        assert_eq!(normalize_mention_name("Bob] Smith"), "Bob Smith"); // bracket -> space, collapsed
        assert_eq!(normalize_mention_name("a\nb"), "a b");
        assert_eq!(normalize_mention_name("  x   y  "), "x y");
        assert_eq!(normalize_mention_name(&"z".repeat(50)).chars().count(), 40);
    }

    #[tokio::test]
    async fn message_stats_aggregates_count_timestamps_and_distinct_days() {
        // The friends-list sortings ride on these stats: total volume, last activity, and the
        // number of distinct days a conversation was active.
        let hub = Hub::new();
        let clock = ManualClock::new(86_400_000); // day 1 (a whole day in ms, so the bucket is 1)
        let mut alice = Server::found(
            hub.join(PeerId::from_u64(1)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(clock.clone()),
            "alice",
        )
        .unwrap();
        alice.open_channel(GENERAL).await.unwrap();

        // An empty conversation → all-zero stats (sorts to the bottom).
        let empty = alice.message_stats(GENERAL);
        assert_eq!(empty.count, 0);
        assert_eq!(empty.active_days, 0);
        assert_eq!(empty.last_ts, 0);

        // Two messages on day 1, one more on day 3 (advance two days).
        alice.send_message(GENERAL, "one").await.unwrap();
        alice.send_message(GENERAL, "two").await.unwrap();
        clock.advance_ms(2 * 86_400_000);
        alice.send_message(GENERAL, "three").await.unwrap();

        let s = alice.message_stats(GENERAL);
        assert_eq!(s.count, 3, "all three messages counted");
        assert_eq!(s.first_ts, 86_400_000);
        assert_eq!(s.last_ts, 3 * 86_400_000);
        assert_eq!(s.active_days, 2, "messages span two distinct UTC days");
    }

    #[tokio::test]
    async fn a_server_survives_a_sealed_store_round_trip() {
        // The full 9f loop: found → post → snapshot → seal to disk → reopen → restore →
        // read the history back, all offline (a fresh transport, no peers).
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(7);

        let mut alice = founder();
        alice.open_channel(GENERAL).await.unwrap();
        alice.send_message(GENERAL, "persist me").await.unwrap();
        alice
            .set_channel_topic(GENERAL, "persist me too")
            .await
            .unwrap();
        let snap = alice.snapshot().unwrap();

        {
            let store = ServerStore::open(dir.path(), b"correct horse", &mut rng).unwrap();
            store.save_server(42, &snap, &mut rng).unwrap();
            store
                .save_registry(
                    &[ServerRecord {
                        id: 42,
                        display_name: "alice".into(),
                        invite: String::new(),
                        is_dm: false,
                    }],
                    &mut rng,
                )
                .unwrap();
        }

        // Reopen the store, reload the server onto a FRESH transport.
        let store = ServerStore::open(dir.path(), b"correct horse", &mut rng).unwrap();
        let reg = store.load_registry().unwrap();
        assert_eq!(reg.len(), 1);
        let bytes = store.load_server(reg[0].id).unwrap();
        let hub = Hub::new();
        let restored = Server::restore(
            &bytes,
            hub.join(PeerId::from_u64(2)),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(2_000)),
            &reg[0].display_name,
        )
        .unwrap();

        // The channel history, topic, display name and roster all survived; read offline.
        let msgs = restored.messages(GENERAL);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "persist me");
        assert_eq!(restored.channel_topic(GENERAL), "persist me too");
        assert_eq!(restored.display_name(), "alice");
        assert_eq!(restored.member_count(), 1);
    }

    #[tokio::test]
    async fn a_customized_profile_survives_a_snapshot_restore() {
        // Regression: a reload must keep the user's customized name/styling. The snapshot preserves
        // the Profile doc; the actor's `spawn` seed is "only if absent", so it leaves a restored
        // profile alone (the bug was an unconditional re-seed reverting the name to the founding one).
        let mut alice = founder();
        alice.open_profiles().await.unwrap();
        alice
            .set_profile(Profile {
                name: "Renamed Alice".into(),
                color: "#ff0000".into(),
                ..Profile::default()
            })
            .await
            .unwrap();
        let me = alice.my_fingerprint();
        let snap = alice.snapshot().unwrap();

        // Restore onto a fresh transport; note the founding display name "alice" is NOT the saved
        // profile name, so a naive re-seed would clobber "Renamed Alice".
        let hub = Hub::new();
        let restored = Server::restore(
            &snap,
            hub.join(PeerId::from_u64(9)),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(2_000)),
            "alice",
        )
        .unwrap();

        let p = restored.profiles();
        assert!(
            p.contains_key(&me),
            "the profile exists post-restore → spawn's guard skips the seed"
        );
        assert_eq!(p[&me].name, "Renamed Alice", "custom name survived reload");
        assert_eq!(p[&me].color, "#ff0000", "custom styling survived reload");
    }

    #[tokio::test]
    async fn a_joiner_converges_on_the_channel_through_the_facade() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice = Server::found(
            hub.join(alice_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
            "alice",
        )
        .unwrap();
        alice.subscribe_control().await.unwrap();
        alice.open_channel(GENERAL).await.unwrap();
        alice.send_message(GENERAL, "welcome!").await.unwrap();
        alice
            .set_channel_topic(GENERAL, "the main room")
            .await
            .unwrap();

        // Bob joins via an invite, over the hub (Alice serves the join with a tick).
        let invite = alice.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let bob_net = hub.join(PeerId::from_u64(2));
        let bob_device = MlsDevice::generate().unwrap();
        let bob_id = bob_device.device_id();
        let (bob, _) = tokio::join!(
            Server::join(
                bob_net,
                bob_device,
                ChaCha20Rng::seed_from_u64(2),
                Box::new(ManualClock::new(1_000)),
                "bob",
                alice_peer,
                &invite,
            ),
            alice.sync_once(),
        );
        let mut bob = bob.unwrap();
        assert_eq!(bob.member_count(), 2);
        assert!(
            alice.members().contains(&bob_id),
            "Bob is in Alice's roster"
        );

        // Bob catches up Alice's channel history over request/response (Alice serves it
        // with a concurrent tick).
        bob.open_channel(GENERAL).await.unwrap();
        let (applied, _) = tokio::join!(
            bob.request_channel_catchup(alice_peer, GENERAL),
            alice.sync_once(),
        );
        assert_eq!(applied.unwrap(), 2, "Bob applied Alice's message + topic");

        let msgs = bob.messages(GENERAL);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "welcome!");
        // Authored by Alice's device fingerprint (the name resolves from her profile).
        assert_eq!(msgs[0].author, alice.my_fingerprint());
        // The topic rides in the same channel document, so catch-up delivers it too.
        assert_eq!(bob.channel_topic(GENERAL), "the main room");

        // Delivery state (D2). Bob demonstrably has "welcome!" now, but nothing Alice holds
        // proves it yet; so her snapshot still reports no delivery, which the UI must render as
        // "unknown" rather than as a failure.
        let snapshot = alice.delivery_snapshot(GENERAL);
        let first_delivery_revision = snapshot.revision;
        assert_eq!(snapshot.states.len(), 1, "only own messages are tracked");
        assert_eq!(snapshot.states[0].id, msgs[0].id);
        assert_eq!(snapshot.states[0].delivered, 0);
        assert!(
            bob.delivery_snapshot(GENERAL).states.is_empty(),
            "Bob has sent nothing, so he has nothing to report"
        );

        // …and *any* member may set it; Bob is not the owner and is not refused.
        assert_ne!(bob.my_role(), Role::Owner);
        bob.set_channel_topic(GENERAL, "bob was here")
            .await
            .unwrap();
        assert_eq!(bob.channel_topic(GENERAL), "bob was here");

        // Bob's topic op necessarily builds on Alice's message, so once it reaches Alice she can
        // prove he holds it.
        assert!(alice.sync_once().await.unwrap());
        assert_eq!(alice.channel_topic(GENERAL), "bob was here");
        let snapshot = alice.delivery_snapshot(GENERAL);
        assert!(snapshot.revision > first_delivery_revision);
        assert_eq!(
            snapshot.states[0].delivered, 1,
            "Bob provably holds the message"
        );
        // `reachable` is the presence count, tracked independently; the in-memory hub models no
        // connect/disconnect, so it stays 0 here even though delivery is proven. The two are not
        // a fraction: a member can hold a message and be offline.
        assert_eq!(snapshot.states[0].reachable, alice.online_members().len());

        // The calendar is its own document (`DocType::Calendar`), caught up exactly like the
        // channel; so an event Alice created reaches the joiner.
        alice.open_calendar().await.unwrap();
        alice
            .create_event("Launch party", "bring cake", 9_000, 0, "")
            .await
            .unwrap();
        bob.open_calendar().await.unwrap();
        let (applied, _) =
            tokio::join!(bob.request_calendar_catchup(alice_peer), alice.sync_once());
        assert!(applied.unwrap() >= 1, "Bob applied Alice's calendar op");
        let events = bob.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "Launch party");
        assert_eq!(events[0].body, "bring cake");
        assert_eq!(events[0].author, alice.my_fingerprint());

        // Any member may create an event, but deletion is the author's or a moderator's: Bob is
        // neither the owner nor an admin, so Alice's event is not his to remove; his own is.
        assert_ne!(bob.my_role(), Role::Owner);
        assert!(
            bob.delete_event(&events[0].id).await.is_err(),
            "a non-author, non-moderator cannot delete someone else's event"
        );
        let bobs = bob
            .create_event("Bob's raid", "", 8_000, 0, "")
            .await
            .unwrap();
        assert_eq!(bob.events().len(), 2);
        bob.delete_event(&bobs).await.unwrap();
        assert_eq!(bob.events().len(), 1, "Bob removed his own event");
    }

    #[tokio::test]
    async fn members_derive_the_same_e2e_media_key() {
        // The crux of E2E voice: every member derives the SAME call media key locally from the MLS
        // group secret; it never travels on the wire; and distinct calls get distinct keys.
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice = Server::found(
            hub.join(alice_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
            "alice",
        )
        .unwrap();
        alice.subscribe_control().await.unwrap();

        let invite = alice.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (bob, _) = tokio::join!(
            Server::join(
                hub.join(PeerId::from_u64(2)),
                MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(2),
                Box::new(ManualClock::new(1_000)),
                "bob",
                alice_peer,
                &invite,
            ),
            alice.sync_once(),
        );
        let bob = bob.unwrap();
        assert_eq!(bob.member_count(), 2);

        let call: u128 = 0x00C0_FFEE;
        let (ka, ea) = alice.media_key(call).unwrap();
        let (kb, eb) = bob.media_key(call).unwrap();
        assert_eq!(ea, eb, "both members are on the same epoch");
        assert_eq!(
            ka, kb,
            "identical 32-byte media key, derived independently on each side"
        );
        assert_ne!(ka, [0u8; 32]);

        // A different call id yields an independent key (domain separation by the call id).
        let (other, _) = alice.media_key(0xBEEF).unwrap();
        assert_ne!(ka, other, "distinct calls get distinct keys");
    }

    #[tokio::test]
    async fn an_embed_file_survives_snapshot_restore_with_the_sealing_blob_store() {
        // Reproduces the "embeds don't survive a restart" path: add a file through the on-disk
        // sealing blob store, snapshot the server, restore it, re-attach the SAME blob store
        // (as the desktop reload does, keyed by the group id), and download it back.
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(3);
        let store = ServerStore::open(dir.path(), b"correct horse", &mut rng).unwrap();

        let mut alice = founder();
        let gid = "test-group"; // a stable key (the bridge uses the hex group id)
        alice.set_blob_store(store.blob_store(gid).unwrap());
        alice.open_files().await.unwrap();
        let cid = alice
            .add_file("pic.png", "image/png", "embed/me", b"PNG-BYTES-xyz")
            .await
            .unwrap();

        let snap = alice.snapshot().unwrap();

        // Restart: restore onto a fresh transport + re-attach the same on-disk blob store.
        let hub = Hub::new();
        let mut restored = Server::restore(
            &snap,
            hub.join(PeerId::from_u64(9)),
            ChaCha20Rng::seed_from_u64(0),
            Box::new(ManualClock::new(1)),
            "alice",
        )
        .unwrap();
        restored.set_blob_store(store.blob_store(gid).unwrap());

        let got = restored.download_file(&cid).await.unwrap();
        assert_eq!(
            got,
            b"PNG-BYTES-xyz".to_vec(),
            "an embedded file downloads + decrypts after a restart"
        );
    }

    #[tokio::test]
    async fn an_uploaded_file_downloads_after_the_vault_is_reopened() {
        // The real restart path: the vault is CLOSED and RE-OPENED (keys re-derived from the
        // passphrase) between uploading a file and downloading it. Reproduces "I uploaded it but
        // get 'file unavailable'": if blob_key isn't stable across re-open, has_blob stays true
        // (the sealed file is on disk) but get_blob can't unseal -> None -> unavailable.
        let dir = tempfile::tempdir().unwrap();
        let key = "g"; // the blob-store dir label (the bridge uses the hex group id)

        let cid;
        let snap;
        {
            let mut rng = ChaCha20Rng::seed_from_u64(7);
            let store = ServerStore::open(dir.path(), b"correct horse", &mut rng).unwrap();
            let mut alice = founder();
            alice.set_blob_store(store.blob_store(key).unwrap());
            alice.open_files().await.unwrap();
            cid = alice
                .add_file("doc.bin", "application/octet-stream", "", b"hello-bytes")
                .await
                .unwrap();
            snap = alice.snapshot().unwrap();
        } // store + alice dropped; simulate the app closing

        // Re-open the vault (re-derive keys) and restore the server, as a restart does.
        let mut rng2 = ChaCha20Rng::seed_from_u64(8);
        let store2 = ServerStore::open(dir.path(), b"correct horse", &mut rng2).unwrap();
        let hub = Hub::new();
        let mut restored = Server::restore(
            &snap,
            hub.join(PeerId::from_u64(9)),
            ChaCha20Rng::seed_from_u64(0),
            Box::new(ManualClock::new(1)),
            "alice",
        )
        .unwrap();
        restored.set_blob_store(store2.blob_store(key).unwrap());

        let got = restored.download_file(&cid).await.unwrap();
        assert_eq!(
            got,
            b"hello-bytes".to_vec(),
            "an uploaded file must still download after the vault is reopened"
        );
    }

    #[tokio::test]
    async fn the_owner_removes_a_member_and_a_non_owner_cannot() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice = Server::found(
            hub.join(alice_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
            "alice",
        )
        .unwrap();
        alice.subscribe_control().await.unwrap();

        let invite = alice.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let bob_device = MlsDevice::generate().unwrap();
        let bob_fp = fingerprint(&bob_device.device_id());
        let (bob, _) = tokio::join!(
            Server::join(
                hub.join(PeerId::from_u64(2)),
                bob_device,
                ChaCha20Rng::seed_from_u64(2),
                Box::new(ManualClock::new(1_000)),
                "bob",
                alice_peer,
                &invite,
            ),
            alice.sync_once(),
        );
        let mut bob = bob.unwrap();
        assert_eq!(alice.member_count(), 2);

        // A non-owner (Bob) cannot remove anyone; the owner cannot remove itself.
        assert!(matches!(
            bob.remove_member(&alice.my_fingerprint()).await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            alice.remove_member(&alice.my_fingerprint()).await,
            Err(AppError::Invalid(_))
        ));

        // The owner removes Bob; the MLS commit drops him from the roster.
        alice.remove_member(&bob_fp).await.unwrap();
        assert_eq!(alice.member_count(), 1, "owner removed Bob");
    }

    #[tokio::test]
    async fn an_owner_deletes_a_file_and_a_member_cannot() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice = Server::found(
            hub.join(alice_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
            "alice",
        )
        .unwrap();
        alice.subscribe_control().await.unwrap();

        let invite = alice.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (bob, _) = tokio::join!(
            Server::join(
                hub.join(PeerId::from_u64(2)),
                MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(2),
                Box::new(ManualClock::new(1_000)),
                "bob",
                alice_peer,
                &invite,
            ),
            alice.sync_once(),
        );
        let mut bob = bob.unwrap();

        alice.open_files().await.unwrap();
        let cid = alice
            .add_file("doc.txt", "text/plain", "", b"hello")
            .await
            .unwrap();
        assert_eq!(alice.files().len(), 1);

        // A plain member cannot delete a file; the role gate rejects before anything else.
        assert!(matches!(
            bob.delete_file(&cid).await,
            Err(AppError::Invalid(_))
        ));
        assert_eq!(alice.files().len(), 1, "the file is still listed");

        // The owner deletes it; the listing is removed.
        alice.delete_file(&cid).await.unwrap();
        assert!(alice.files().is_empty(), "owner deleted the file");

        // Deleting a now-absent file errors rather than silently succeeding.
        assert!(matches!(
            alice.delete_file(&cid).await,
            Err(AppError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn a_member_profile_is_set_and_read_back() {
        let mut alice = founder();
        alice.open_profiles().await.unwrap();
        assert!(alice.profiles().is_empty());

        let p = Profile {
            name: "Alice".into(),
            color: "#ff5577".into(),
            font: "serif".into(),
            effect: "rainbow".into(),
            description: "the founder".into(),
            bubble: "linear-gradient(90deg,#f06,#09f)".into(),
            avatar: vec![0xff, 0xd8, 0xff, 0x00, 1, 2, 3], // stand-in JPEG bytes
            banner: Vec::new(),
        };
        alice.set_profile(p.clone()).await.unwrap();

        let profiles = alice.profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles.get(&alice.my_fingerprint()), Some(&p));
    }

    #[tokio::test]
    async fn a_profile_banner_round_trips_by_content_address() {
        let mut alice = founder();
        alice.open_profiles().await.unwrap();

        // The banner rides by content address like the avatar: the doc carries only the CID,
        // the bytes come back resolved against the local blob store.
        let p = Profile {
            name: "Alice".into(),
            avatar: vec![0xff, 0xd8, 0xff, 0x00, 1, 2, 3], // stand-in JPEG bytes
            banner: vec![0xABu8; 4096],                    // stand-in wide card image
            ..Default::default()
        };
        alice.set_profile(p.clone()).await.unwrap();

        let profiles = alice.profiles();
        assert_eq!(profiles.len(), 1);
        let got = profiles.get(&alice.my_fingerprint()).expect("own profile");
        assert_eq!(got, &p);
        assert_eq!(got.banner, p.banner, "the banner resolved back to bytes");

        // Clearing the banner leaves the avatar alone.
        let cleared = Profile {
            banner: Vec::new(),
            ..p.clone()
        };
        alice.set_profile(cleared.clone()).await.unwrap();
        let after = alice.profiles();
        let got = after.get(&alice.my_fingerprint()).expect("own profile");
        assert!(got.banner.is_empty(), "the banner was cleared");
        assert_eq!(got.avatar, p.avatar, "the avatar survived");
    }

    #[tokio::test]
    async fn a_server_livery_is_published_and_read_back() {
        let mut alice = founder();
        alice.open_livery().await.unwrap();
        alice.open_roles().await.unwrap();
        assert_eq!(alice.livery(), Livery::default());

        let l = Livery {
            preset: "aurum".into(),
            accent: "#ffcc00".into(),
            tokens: HashMap::from([("--accent-hi".to_string(), "#ffe680".to_string())]),
            icon: String::new(),
            cursor: String::new(),
        };
        alice.set_livery(l.clone()).await.unwrap();
        assert_eq!(alice.livery(), l);

        // Removing the livery clears the token map too (it is replaced wholesale).
        alice.set_livery(Livery::default()).await.unwrap();
        assert_eq!(alice.livery(), Livery::default());

        // Oversized values are rejected, like an over-large avatar.
        let too_long = Livery {
            preset: "x".repeat(MAX_LIVERY_PRESET_BYTES + 1),
            ..Default::default()
        };
        assert!(matches!(
            alice.set_livery(too_long).await,
            Err(AppError::Invalid(_))
        ));

        // --- the shared server icon (its own command, its own lifetime) --------
        let icon = B64.encode([0xff, 0xd8, 0xff, 0x00, 1, 2, 3]); // stand-in JPEG bytes
        alice.set_server_icon(icon.clone()).await.unwrap();
        assert_eq!(alice.livery().icon, icon, "the icon reads back");

        // Publishing colours must NOT resend or drop the image: set_livery is a
        // read-modify-write that carries the stored icon through untouched.
        alice.set_livery(l.clone()).await.unwrap();
        let after = alice.livery();
        assert_eq!(after.icon, icon, "the icon survived a colour publish");
        assert_eq!(after.preset, l.preset);
        assert_eq!(after.accent, l.accent);
        assert_eq!(after.tokens, l.tokens);

        // …and an empty livery does not clear it either (only set_server_icon does).
        alice.set_livery(Livery::default()).await.unwrap();
        assert_eq!(
            alice.livery().icon,
            icon,
            "removing the livery keeps the icon"
        );

        // `""` clears the icon.
        alice.set_server_icon(String::new()).await.unwrap();
        assert_eq!(alice.livery(), Livery::default(), "the icon is gone");

        // An over-large icon is rejected on decoded size, like an over-large avatar…
        let too_big = B64.encode(vec![0u8; MAX_SERVER_ICON_BYTES + 1]);
        assert!(matches!(
            alice.set_server_icon(too_big).await,
            Err(AppError::Invalid(_))
        ));
        // …as is anything that is not base64 at all.
        assert!(matches!(
            alice.set_server_icon("not base64!!".into()).await,
            Err(AppError::Invalid(_))
        ));
        assert_eq!(
            alice.livery(),
            Livery::default(),
            "no rejected write landed"
        );

        // --- the shared server cursor (mirrors the icon, independent lifetime) ---
        let cursor = B64.encode([0x89, b'P', b'N', b'G', 9, 8, 7]); // stand-in PNG bytes
        alice.set_server_cursor(cursor.clone()).await.unwrap();
        assert_eq!(alice.livery().cursor, cursor, "the cursor reads back");

        // Publishing colours carries the cursor through untouched, exactly like the icon…
        alice.set_livery(l.clone()).await.unwrap();
        let after = alice.livery();
        assert_eq!(after.cursor, cursor, "the cursor survived a colour publish");
        assert_eq!(after.preset, l.preset);
        // …and an empty livery does not clear it either.
        alice.set_livery(Livery::default()).await.unwrap();
        assert_eq!(
            alice.livery().cursor,
            cursor,
            "removing the livery keeps the cursor"
        );

        // The two images have wholly independent lifetimes, both ways round: writing or
        // clearing one never disturbs the other.
        alice.set_server_icon(icon.clone()).await.unwrap();
        let after = alice.livery();
        assert_eq!(after.cursor, cursor, "setting the icon kept the cursor");
        assert_eq!(after.icon, icon);
        alice.set_server_cursor(cursor.clone()).await.unwrap();
        assert_eq!(
            alice.livery().icon,
            icon,
            "setting the cursor kept the icon"
        );
        alice.set_server_icon(String::new()).await.unwrap();
        assert_eq!(
            alice.livery().cursor,
            cursor,
            "clearing the icon kept the cursor"
        );

        // `""` clears the cursor.
        alice.set_server_cursor(String::new()).await.unwrap();
        assert_eq!(alice.livery(), Livery::default(), "the cursor is gone");

        // An over-large cursor is rejected on its own (tighter) decoded-size cap…
        let too_big = B64.encode(vec![0u8; MAX_SERVER_CURSOR_BYTES + 1]);
        assert!(matches!(
            alice.set_server_cursor(too_big).await,
            Err(AppError::Invalid(_))
        ));
        // …as is anything that is not base64 at all.
        assert!(matches!(
            alice.set_server_cursor("not base64!!".into()).await,
            Err(AppError::Invalid(_))
        ));
        assert_eq!(
            alice.livery(),
            Livery::default(),
            "no rejected cursor write landed"
        );
    }

    #[tokio::test]
    async fn a_member_badge_is_assigned_and_read_back() {
        let mut alice = founder();
        alice.open_badges().await.unwrap();
        alice.open_roles().await.unwrap();
        assert!(alice.badges().is_empty());

        let fp = alice.my_fingerprint();
        alice
            .set_member_badge(fp.clone(), "ARTIST".into(), "#3fb8af".into())
            .await
            .unwrap();
        assert_eq!(
            alice.badges().get(&fp),
            Some(&MemberBadge {
                label: "ARTIST".into(),
                color: "#3fb8af".into(),
            })
        );

        // Re-assigning replaces that member's badge (last-writer-wins per member).
        alice
            .set_member_badge(fp.clone(), "WRITER".into(), "#ffcc00".into())
            .await
            .unwrap();
        let badges = alice.badges();
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[&fp].label, "WRITER");
        assert_eq!(badges[&fp].color, "#ffcc00");

        // An empty label removes the entry entirely.
        alice
            .set_member_badge(fp.clone(), String::new(), String::new())
            .await
            .unwrap();
        assert!(alice.badges().is_empty());

        // Oversized values are rejected, like an over-long livery preset.
        assert!(matches!(
            alice
                .set_member_badge(
                    fp.clone(),
                    "x".repeat(MAX_BADGE_LABEL_BYTES + 1),
                    String::new()
                )
                .await,
            Err(AppError::Invalid(_))
        ));

        // Role words are reserved, however they are cased or padded; a custom badge must
        // never be able to read as a built-in role.
        for label in ["admin", " ADMIN ", "Owner", "mod", "Moderator"] {
            assert!(
                matches!(
                    alice
                        .set_member_badge(fp.clone(), label.into(), String::new())
                        .await,
                    Err(AppError::Invalid(_))
                ),
                "reserved label {label:?} must be rejected"
            );
        }
        assert!(alice.badges().is_empty(), "no rejected write landed");

        // The entry count is capped: filling the map is fine…
        for i in 0..MAX_BADGES {
            alice
                .set_member_badge(format!("fp{i}"), "ARTIST".into(), "#3fb8af".into())
                .await
                .unwrap();
        }
        assert_eq!(alice.badges().len(), MAX_BADGES);
        // …one more *member* is rejected…
        assert!(matches!(
            alice
                .set_member_badge("overflow".into(), "ARTIST".into(), String::new())
                .await,
            Err(AppError::Invalid(_))
        ));
        // …while re-badging or clearing an existing member still works at the cap.
        alice
            .set_member_badge("fp0".into(), "WRITER".into(), String::new())
            .await
            .unwrap();
        alice
            .set_member_badge("fp0".into(), String::new(), String::new())
            .await
            .unwrap();
        assert_eq!(alice.badges().len(), MAX_BADGES - 1);
    }

    #[tokio::test]
    async fn an_oversize_avatar_is_rejected() {
        let mut alice = founder();
        alice.open_profiles().await.unwrap();
        let p = Profile {
            name: "Alice".into(),
            avatar: vec![0u8; MAX_AVATAR_BYTES + 1],
            ..Default::default()
        };
        assert!(matches!(
            alice.set_profile(p).await,
            Err(AppError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn an_oversize_banner_is_rejected() {
        let mut alice = founder();
        alice.open_profiles().await.unwrap();
        let p = Profile {
            name: "Alice".into(),
            banner: vec![0u8; MAX_BANNER_BYTES + 1],
            ..Default::default()
        };
        assert!(matches!(
            alice.set_profile(p).await,
            Err(AppError::Invalid(_))
        ));
        // A banner at the cap is fine: it gets a bigger budget than an avatar.
        let ok = Profile {
            name: "Alice".into(),
            banner: vec![0u8; MAX_BANNER_BYTES],
            ..Default::default()
        };
        alice.set_profile(ok).await.unwrap();
        assert_eq!(
            alice.profiles()[&alice.my_fingerprint()].banner.len(),
            MAX_BANNER_BYTES
        );
    }

    #[tokio::test]
    async fn a_file_is_added_listed_and_downloaded_locally() {
        let mut alice = founder();
        alice.open_files().await.unwrap();
        assert!(alice.files().is_empty());

        let data = b"the quick brown fox".to_vec();
        let cid = alice
            .add_file("notes.txt", "text/plain", "docs/sub", &data)
            .await
            .unwrap();

        let files = alice.files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "notes.txt");
        assert_eq!(files[0].size, data.len() as u64);
        assert_eq!(files[0].mime, "text/plain");
        assert_eq!(files[0].cid, cid.as_bytes().to_vec());
        assert_eq!(files[0].author, alice.my_fingerprint());
        assert_eq!(files[0].path, "docs/sub", "the folder path round-trips");

        // The uploader already holds the bytes.
        assert_eq!(alice.download_file(&cid).await.unwrap(), data);
    }

    #[tokio::test]
    async fn storage_health_detects_missing_chunks_and_authenticated_repair_recovers_them() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let clock = ManualClock::new(T0);
        let mut alice = founder_on(&hub, alice_peer, &clock, 41);
        alice.subscribe_control().await.unwrap();
        alice.open_files().await.unwrap();
        let invite = alice.mint_invite([4u8; 16], u64::MAX, vec![]).unwrap();
        let (bob, _) = tokio::join!(
            Server::join(
                hub.join(PeerId::from_u64(2)),
                MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(42),
                Box::new(clock.clone()),
                "bob",
                alice_peer,
                &invite,
            ),
            alice.sync_once(),
        );
        let mut bob = bob.unwrap();
        bob.open_files().await.unwrap();

        alice
            .add_file("repair.txt", "text/plain", "", b"recoverable bytes")
            .await
            .unwrap();
        let _ = tokio::join!(bob.request_files_catchup(alice_peer), alice.sync_once());

        let missing = bob.storage_health();
        assert_eq!(missing.listed_files, 1);
        assert_eq!(missing.referenced_chunks, 1);
        assert_eq!(missing.missing_chunks, 1);
        assert_eq!(missing.verified_chunks, 0);

        let (repair, _) = tokio::join!(bob.repair_storage(), alice.sync_once());
        let repair = repair.unwrap();
        assert_eq!(repair.attempted_chunks, 1);
        assert_eq!(repair.recovered_chunks, 1);
        assert_eq!(repair.health.missing_chunks, 0);
        assert_eq!(repair.health.unreadable_chunks, 0);
        assert_eq!(repair.health.verified_chunks, 1);
        assert_eq!(repair.health.verified_manifest_versions.len(), 1);
    }

    #[tokio::test]
    async fn storage_health_binds_completeness_to_the_exact_chunk_reference() {
        let mut alice = founder();
        alice.open_files().await.unwrap();
        alice
            .add_file(
                "safe.png",
                "image/png",
                "safe",
                b"authenticated image bytes",
            )
            .await
            .unwrap();
        let original = alice.files().remove(0);
        let original_version = file_manifest_version(&original.file_ref);
        let original_manifest = FileManifest::decode_or_legacy(&original.file_ref).unwrap();

        // Repeat the exact ciphertext address but alter the wrapped key. Presence checks report
        // both rows as held; only an exact-FileRef open distinguishes this unreadable claim.
        let mut hostile_manifest = original_manifest.clone();
        hostile_manifest.chunks[0].wrapped_key.ciphertext[0] ^= 1;
        let hostile_ref = hostile_manifest.encode();
        let hostile_version = file_manifest_version(&hostile_ref);
        let author = alice.my_fingerprint();
        let (public_key, signature) = alice
            .file_entry_attestation("hostile.png", &author, "hostile", &hostile_ref)
            .unwrap();
        alice
            .sync
            .post(DocType::FileIndex, FILE_INDEX_DOC, |doc| {
                write_file_entry(
                    doc,
                    "hostile.png",
                    &author,
                    "hostile",
                    &hostile_ref,
                    FileExpiry::Never,
                    Some((&public_key, &signature)),
                )
            })
            .await
            .unwrap();

        let snapshot = alice.storage_snapshot();
        assert_eq!(snapshot.files.files.len(), 2);
        assert!(snapshot
            .files
            .files
            .iter()
            .all(|listing| listing.held_chunks == listing.total_chunks));
        assert_eq!(snapshot.health.referenced_chunks, 1);
        assert_eq!(snapshot.health.verified_chunks, 0);
        assert_eq!(snapshot.health.unreadable_chunks, 1);
        assert!(snapshot
            .health
            .verified_manifest_versions
            .contains(&original_version));
        assert!(!snapshot
            .health
            .verified_manifest_versions
            .contains(&hostile_version));
    }

    #[tokio::test]
    async fn storage_health_bounds_same_cid_exact_reference_work() {
        let mut alice = founder();
        alice.open_files().await.unwrap();
        alice
            .add_file("seed.bin", "application/octet-stream", "", b"held")
            .await
            .unwrap();
        let seed = alice.files().remove(0);
        let seed_manifest = FileManifest::decode_or_legacy(&seed.file_ref).unwrap();
        let seed_chunk = seed_manifest.chunks[0].clone();
        let mut hostile_entries = Vec::with_capacity(MAX_FILE_ENTRIES);

        // Exercise the materialized index's full 256 rows × 32 chunks. Every chunk points at the
        // same held ciphertext but carries a distinct wrapped-key claim. Without the per-CID cap,
        // a storage-pane read would perform 8,192 opens of the same (potentially 8 MiB) blob while
        // the server actor could process no messages or sync work.
        for row in 0..MAX_FILE_ENTRIES {
            let mut chunks = Vec::with_capacity(catcoms_storage::MAX_CHUNKS as usize);
            for index in 0..catcoms_storage::MAX_CHUNKS as usize {
                let ordinal = row * catcoms_storage::MAX_CHUNKS as usize + index;
                let mut chunk = seed_chunk.clone();
                chunk.size = CHUNK_BYTES as u64;
                chunk.wrapped_key.ciphertext[..4].copy_from_slice(&(ordinal as u32).to_be_bytes());
                chunks.push(chunk);
            }
            let manifest = FileManifest {
                plaintext_cid: Cid::of(&(row as u64).to_be_bytes()),
                total_size: MAX_FILE_BYTES as u64,
                mime: "application/octet-stream".into(),
                chunks,
            };
            let mut entry = seed.clone();
            entry.name = format!("hostile-{row}.bin");
            entry.size = MAX_FILE_BYTES as u64;
            entry.cid = manifest.plaintext_cid.as_bytes().to_vec();
            entry.file_ref = manifest.encode();
            hostile_entries.push(entry);
        }

        let mut open_count = 0usize;
        let health =
            alice.storage_health_for_entries_with(&hostile_entries, |_ciphertext, _file_ref| {
                open_count += 1;
                true
            });
        assert!(open_count <= MAX_FILE_REF_VARIANTS_PER_CID);
        assert_eq!(
            open_count, 0,
            "an over-cap CID fails closed before decryption"
        );
        assert_eq!(health.referenced_chunks, 1);
        assert_eq!(health.unreadable_chunks, 1);
        assert_eq!(health.verified_chunks, 0);
        assert!(health.verified_manifest_versions.is_empty());
    }

    #[tokio::test]
    async fn storage_repair_leaves_over_cap_exact_references_visibly_unrepairable() {
        let mut alice = founder();
        alice.open_files().await.unwrap();
        alice
            .add_file("seed.bin", "application/octet-stream", "", b"held")
            .await
            .unwrap();
        let seed = alice.files().remove(0);
        let seed_manifest = FileManifest::decode_or_legacy(&seed.file_ref).unwrap();
        let author = alice.my_fingerprint();

        // The original plus four distinct variants crosses the cap. Fetching cannot help: all
        // five claims name the same already-held ciphertext, so repair must do no peer work and
        // must retain the unreadable verdict instead of reporting a false recovery.
        for index in 0..MAX_FILE_REF_VARIANTS_PER_CID {
            let mut hostile = seed_manifest.clone();
            hostile.chunks[0].wrapped_key.ciphertext[..4]
                .copy_from_slice(&(index as u32 + 1).to_be_bytes());
            let hostile_ref = hostile.encode();
            let name = format!("hostile-{index}.bin");
            let (public_key, signature) = alice
                .file_entry_attestation(&name, &author, "hostile", &hostile_ref)
                .unwrap();
            alice
                .sync
                .post(DocType::FileIndex, FILE_INDEX_DOC, |doc| {
                    write_file_entry(
                        doc,
                        &name,
                        &author,
                        "hostile",
                        &hostile_ref,
                        FileExpiry::Never,
                        Some((&public_key, &signature)),
                    )
                })
                .await
                .unwrap();
        }

        let before = alice.storage_health();
        assert_eq!(before.unreadable_chunks, 1);
        assert_eq!(before.verified_chunks, 0);
        let repair = alice.repair_storage().await.unwrap();
        assert_eq!(repair.attempted_chunks, 0);
        assert_eq!(repair.recovered_chunks, 0);
        assert_eq!(repair.health.unreadable_chunks, 1);
        assert_eq!(repair.health.verified_chunks, 0);
        assert!(repair.health.verified_manifest_versions.is_empty());
    }

    #[tokio::test]
    async fn the_founder_is_owner_and_gates_invites_and_admin_grants() {
        let mut alice = founder();
        alice.open_roles().await.unwrap();

        // The founder is the owner (the MLS designated committer) and may invite.
        assert!(alice.is_owner());
        assert_eq!(alice.my_role(), Role::Owner);
        assert!(alice.my_role().can_invite());
        assert!(alice.mint_invite([1u8; 16], u64::MAX, vec![]).is_ok());

        // The owner is reported as owner in the roster roles.
        assert!(
            alice
                .roles()
                .get(&alice.my_fingerprint())
                .map(String::as_str)
                == Some("owner")
        );
        // The owner may write a grant, but a grant for a non-member never resolves to admin;
        // admin_set is filtered to current members, so a stale/bogus fp can't become admin.
        alice.set_admin("deadbeef", true).await.unwrap();
        assert_eq!(alice.role_of("deadbeef"), Role::Member);
    }

    #[test]
    fn read_published_roster_only_accepts_the_current_owners_signature() {
        use automerge::transaction::Transactable;
        use catcoms_sync::{encode_roster, roster_payload, ROSTER_KEY};
        let owner = MlsDevice::generate().unwrap();
        let other = MlsDevice::generate().unwrap();
        let gid = b"group-xyz".to_vec();
        let owner_id = owner.device_id();
        let fps = vec!["aaaa1111".to_string(), "bbbb2222".to_string()];

        let mk_roster = |dev: &MlsDevice, group: &[u8], gen: u64, fps: &[String]| {
            let sig = dev.sign(&roster_payload(group, gen, fps)).unwrap();
            encode_roster(gen, &dev.public_key_bytes(), fps, &sig)
        };

        // A valid owner-signed roster.
        let mut doc = AutoCommit::new();
        doc.put(
            ROOT,
            ROSTER_KEY,
            ScalarValue::Bytes(mk_roster(&owner, &gid, 1, &fps)),
        )
        .unwrap();
        let admins = read_published_roster(&doc, &gid, &owner_id).expect("owner roster read");
        assert!(
            admins.contains("aaaa1111") && admins.contains("bbbb2222"),
            "the owner-signed roster's admins are read"
        );

        // Forged: signed by a non-owner device.
        let mut doc2 = AutoCommit::new();
        doc2.put(
            ROOT,
            ROSTER_KEY,
            ScalarValue::Bytes(mk_roster(&other, &gid, 1, &fps)),
        )
        .unwrap();
        assert!(
            read_published_roster(&doc2, &gid, &owner_id).is_none(),
            "a non-owner-signed roster is rejected (cannot forge admin)"
        );

        // Owner-signed but bound to a DIFFERENT group (cross-group replay).
        let mut doc3 = AutoCommit::new();
        doc3.put(
            ROOT,
            ROSTER_KEY,
            ScalarValue::Bytes(mk_roster(&owner, b"other-group", 1, &fps)),
        )
        .unwrap();
        assert!(
            read_published_roster(&doc3, &gid, &owner_id).is_none(),
            "a roster bound to a different group is rejected"
        );

        // Absent roster → None (fail-closed).
        assert!(
            read_published_roster(&AutoCommit::new(), &gid, &owner_id).is_none(),
            "an absent roster reads as no admins (fail-closed)"
        );
    }

    #[test]
    fn folder_paths_are_normalized_and_cannot_escape() {
        assert_eq!(normalize_path(""), "");
        assert_eq!(normalize_path("/docs/"), "docs");
        assert_eq!(normalize_path("a//b/./c"), "a/b/c");
        assert_eq!(normalize_path("../../etc/passwd"), "etc/passwd");
        assert_eq!(normalize_path("embed/ fp /x"), "embed/fp/x");
    }

    #[tokio::test]
    async fn a_status_is_posted_and_read() {
        let mut alice = founder();
        alice.open_status().await.unwrap();
        assert!(alice.statuses().is_empty());
        alice.post_status("server is live").await.unwrap();
        let feed = alice.statuses();
        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].text, "server is live");
        assert_eq!(feed[0].author, alice.my_fingerprint());
        assert_eq!(feed[0].ts, 1_000);
    }

    /// A founder and a joiner on one server, both with the status feed open. Every rule the feed
    /// adds is about *who* is acting, so a second identity is the fixture rather than a detail of
    /// it; the joiner is a plain member, which is the role the rules actually bite on.
    async fn status_duo(
        clock: &ManualClock,
    ) -> (
        Server<MemNetwork, ChaCha20Rng>,
        Server<MemNetwork, ChaCha20Rng>,
    ) {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice = Server::found(
            hub.join(alice_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(clock.clone()),
            "alice",
        )
        .unwrap();
        alice.subscribe_control().await.unwrap();
        let invite = alice.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (bob, _) = tokio::join!(
            Server::join(
                hub.join(PeerId::from_u64(2)),
                MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(2),
                Box::new(clock.clone()),
                "bob",
                alice_peer,
                &invite,
            ),
            alice.sync_once(),
        );
        let mut bob = bob.unwrap();
        alice.open_status().await.unwrap();
        bob.open_status().await.unwrap();
        (alice, bob)
    }

    /// The id of the feed post reading `text`. Ids are random, and a merged list is not obliged to
    /// keep the order two peers appended in, so the tests address a post by what it says.
    fn status_id(server: &Server<MemNetwork, ChaCha20Rng>, text: &str) -> String {
        server
            .statuses()
            .into_iter()
            .find(|m| m.text == text)
            .unwrap_or_else(|| panic!("no status post reading {text:?}"))
            .id
    }

    #[tokio::test]
    async fn the_status_feed_is_owner_only_until_a_member_is_let_in() {
        let clock = ManualClock::new(T0);
        let (mut alice, mut bob) = status_duo(&clock).await;
        assert!(
            !alice.status_members_may_post(),
            "a feed nobody has configured is the server announcing itself"
        );

        assert!(matches!(
            bob.post_status("i have opinions").await,
            Err(AppError::Invalid(_))
        ));
        drain_sync(&mut alice).await;
        assert!(
            alice.statuses().is_empty(),
            "a refused post reaches the shared document not at all"
        );
        alice.post_status("we open at eight").await.unwrap();

        // A member cannot let themselves in; the owner can, and the answer is shared state.
        assert!(matches!(
            bob.set_status_members_may_post(true).await,
            Err(AppError::Invalid(_))
        ));
        alice.set_status_members_may_post(true).await.unwrap();
        drain_sync(&mut bob).await;
        assert!(bob.status_members_may_post());
        bob.post_status("i have opinions").await.unwrap();
        assert_eq!(bob.statuses().len(), 2);

        // Closing it again refuses the next one; the posts already made stay.
        alice.set_status_members_may_post(false).await.unwrap();
        drain_sync(&mut bob).await;
        assert!(matches!(
            bob.post_status("more opinions").await,
            Err(AppError::Invalid(_))
        ));
        assert_eq!(bob.statuses().len(), 2);
    }

    #[tokio::test]
    async fn a_status_post_is_reworded_only_by_the_member_who_wrote_it() {
        let clock = ManualClock::new(T0);
        let (mut alice, mut bob) = status_duo(&clock).await;
        alice.post_status("we open at eight").await.unwrap();
        drain_sync(&mut bob).await;
        let id = status_id(&alice, "we open at eight");

        // Bob can read it and still may not reword it; nor could an owner reword his, which is
        // why moderation removes a post rather than editing one.
        assert!(matches!(
            bob.edit_status(&id, "we open at nine").await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            alice.edit_status("no-such-post", "hello").await,
            Err(AppError::Invalid(_))
        ));

        clock.advance_ms(1_000);
        alice.edit_status(&id, "we open at nine").await.unwrap();
        assert_eq!(alice.statuses()[0].text, "we open at nine");
        assert_eq!(alice.statuses()[0].edited, T0 + 1_000);

        // Re-saving the same words is dropped, so the "edited" stamp does not creep.
        clock.advance_ms(1_000);
        alice.edit_status(&id, "we open at nine").await.unwrap();
        assert_eq!(alice.statuses()[0].edited, T0 + 1_000);

        drain_sync(&mut bob).await;
        assert_eq!(bob.statuses()[0].text, "we open at nine");
    }

    #[test]
    fn concurrent_first_status_posts_survive_a_merge() {
        // Neither fork has seen a status container from the other. The legacy shared-list layout
        // lost one side in precisely this case; disjoint root keys must preserve both posts.
        let mut base = AutoCommit::new();
        let mut alice = base.fork();
        let mut bob = base.fork();
        append_status_message(
            &mut alice,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "alice",
            "owner notice",
            T0,
        )
        .unwrap();
        append_status_message(
            &mut bob,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "bob",
            "member reply",
            T0,
        )
        .unwrap();

        alice.merge(&mut bob).unwrap();
        assert_eq!(
            read_status_messages(&alice)
                .into_iter()
                .map(|post| post.text)
                .collect::<Vec<_>>(),
            vec!["owner notice".to_string(), "member reply".to_string()]
        );
    }

    #[test]
    fn conflicting_legacy_status_lists_are_recovered_and_independently_mutable() {
        let alice_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let bob_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let mut base = AutoCommit::new();
        let mut alice = base.fork();
        let mut bob = base.fork();
        append_message(&mut alice, alice_id, "alice", "old notice", T0, "").unwrap();
        append_message(&mut bob, bob_id, "bob", "old reply", T0, "").unwrap();
        alice.merge(&mut bob).unwrap();
        assert_eq!(read_status_messages(&alice).len(), 2);

        for (id, text, reactor) in [
            (alice_id, "updated notice", "carol"),
            (bob_id, "updated reply", "dave"),
        ] {
            assert!(edit_status_message_in_doc(&mut alice, id, text, T0 + 1).unwrap());
            assert!(toggle_status_reaction_in_doc(&mut alice, id, "ok", reactor).unwrap());
            assert!(set_status_pin_in_doc(&mut alice, id, true).unwrap());
        }
        let posts = read_status_messages(&alice);
        assert_eq!(posts.len(), 2);
        assert!(posts.iter().all(|post| post.edited == T0 + 1));
        assert!(posts.iter().all(|post| post.pinned));
        assert_eq!(posts[0].text, "updated notice");
        assert_eq!(posts[1].text, "updated reply");

        assert!(delete_status_message_in_doc(&mut alice, alice_id).unwrap());
        assert_eq!(read_status_messages(&alice)[0].id, bob_id);
        assert!(delete_status_message_in_doc(&mut alice, bob_id).unwrap());
        assert!(read_status_messages(&alice).is_empty());
    }

    #[tokio::test]
    async fn an_ambiguous_status_id_is_hidden_and_every_public_mutation_fails_closed() {
        let mut alice = founder();
        alice.open_status().await.unwrap();
        let id = "cccccccccccccccccccccccccccccccc";
        alice
            .sync
            .post(DocType::Status, STATUS_DOC, |doc| {
                append_message(doc, id, "alice", "legacy Alice post", T0, "")?;
                append_status_message(doc, id, "bob", "keyed Bob post", T0)
            })
            .await
            .unwrap();
        assert!(
            alice.statuses().is_empty(),
            "an ambiguous id is not actionable UI state"
        );

        let before: Vec<ChatMessage> =
            status_post_candidates(alice.sync.doc(DocType::Status, STATUS_DOC).unwrap().doc())
                .unwrap()
                .into_iter()
                .map(|candidate| candidate.post)
                .collect();
        assert!(alice.edit_status(id, "changed").await.is_err());
        assert!(alice.delete_status(id).await.is_err());
        assert!(alice.toggle_status_reaction(id, "ok").await.is_err());
        assert!(alice.set_status_pin(id, true).await.is_err());
        let after: Vec<ChatMessage> =
            status_post_candidates(alice.sync.doc(DocType::Status, STATUS_DOC).unwrap().doc())
                .unwrap()
                .into_iter()
                .map(|candidate| candidate.post)
                .collect();
        assert_eq!(
            after, before,
            "authorization failure must leave both objects untouched"
        );
    }

    #[test]
    fn an_empty_keyed_status_id_is_inert() {
        let mut doc = AutoCommit::new();
        let msg = doc
            .put_object(ROOT, STATUS_POST_PREFIX, ObjType::Map)
            .unwrap();
        doc.put(&msg, MSG_ID, "").unwrap();
        doc.put(&msg, AUTHOR, "mallory").unwrap();
        doc.put(&msg, TEXT, "unaddressable").unwrap();
        doc.put(&msg, TS, T0 as i64).unwrap();

        assert!(read_status_messages(&doc).is_empty());
        assert!(!edit_status_message_in_doc(&mut doc, "", "changed", T0 + 1).unwrap());
        assert!(!toggle_status_reaction_in_doc(&mut doc, "", "ok", "mallory").unwrap());
        assert!(!set_status_pin_in_doc(&mut doc, "", true).unwrap());
        assert!(!delete_status_message_in_doc(&mut doc, "").unwrap());
        assert_eq!(str_field(&doc, &msg, TEXT), "unaddressable");
    }

    #[tokio::test]
    async fn a_status_post_is_deleted_by_its_author_or_a_moderator_and_nobody_else() {
        let clock = ManualClock::new(T0);
        let (mut alice, mut bob) = status_duo(&clock).await;
        alice.set_status_members_may_post(true).await.unwrap();
        drain_sync(&mut bob).await;

        alice.post_status("the owner's notice").await.unwrap();
        bob.post_status("bob's first").await.unwrap();
        bob.post_status("bob's second").await.unwrap();
        drain_sync(&mut alice).await;
        drain_sync(&mut bob).await;
        assert_eq!(bob.statuses().len(), 3);

        let owners = status_id(&bob, "the owner's notice");
        let first = status_id(&bob, "bob's first");
        let second = status_id(&bob, "bob's second");

        assert!(matches!(
            bob.delete_status(&owners).await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            bob.delete_status("no-such-post").await,
            Err(AppError::Invalid(_))
        ));
        bob.delete_status(&first).await.unwrap();
        // The owner may take down a member's post; that is the moderation route.
        alice.delete_status(&second).await.unwrap();

        drain_sync(&mut alice).await;
        drain_sync(&mut bob).await;
        assert_eq!(alice.statuses().len(), 1);
        assert_eq!(alice.statuses()[0].id, owners);
        assert_eq!(bob.statuses().len(), 1);
    }

    #[tokio::test]
    async fn a_status_reaction_goes_on_and_comes_off_again() {
        let clock = ManualClock::new(T0);
        let (mut alice, mut bob) = status_duo(&clock).await;
        alice.post_status("we open at eight").await.unwrap();
        drain_sync(&mut bob).await;
        let id = status_id(&alice, "we open at eight");

        // Reacting is not posting: the member a closed feed refuses may still answer it.
        assert!(!bob.status_members_may_post());
        bob.toggle_status_reaction(&id, "🎉").await.unwrap();
        let feed = bob.statuses();
        assert_eq!(feed[0].reactions.len(), 1);
        assert_eq!(feed[0].reactions[0].emoji, "🎉");
        assert_eq!(feed[0].reactions[0].by, vec![bob.my_fingerprint()]);
        drain_sync(&mut alice).await;
        assert_eq!(
            alice.statuses()[0].reactions[0].by,
            vec![bob.my_fingerprint()]
        );

        bob.toggle_status_reaction(&id, "🎉").await.unwrap();
        assert!(bob.statuses()[0].reactions.is_empty());

        // The trust-boundary rules chat applies apply here too: a post that exists, and an emoji
        // that cannot smuggle the flat key's separator onto the message map.
        assert!(matches!(
            bob.toggle_status_reaction("no-such-post", "🎉").await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            bob.toggle_status_reaction(&id, "a\u{1f}b").await,
            Err(AppError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn pinning_a_status_post_is_owner_or_admin_only() {
        let clock = ManualClock::new(T0);
        let (mut alice, mut bob) = status_duo(&clock).await;
        alice.post_status("we open at eight").await.unwrap();
        alice.post_status("and close at six").await.unwrap();
        drain_sync(&mut bob).await;
        let first = status_id(&alice, "we open at eight");
        let second = status_id(&alice, "and close at six");

        assert!(matches!(
            bob.set_status_pin(&first, true).await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            alice.set_status_pin("no-such-post", true).await,
            Err(AppError::Invalid(_))
        ));

        // Several announcements may be held above the scroll at once, as in a channel.
        alice.set_status_pin(&first, true).await.unwrap();
        alice.set_status_pin(&second, true).await.unwrap();
        assert!(alice.statuses().iter().all(|m| m.pinned));

        alice.set_status_pin(&first, true).await.unwrap(); // redundant; posts nothing
        alice.set_status_pin(&first, false).await.unwrap();
        let feed = alice.statuses();
        assert!(!feed.iter().find(|m| m.id == first).unwrap().pinned);
        assert!(feed.iter().find(|m| m.id == second).unwrap().pinned);

        drain_sync(&mut bob).await;
        let bobs = bob.statuses();
        assert!(!bobs.iter().find(|m| m.id == first).unwrap().pinned);
        assert!(bobs.iter().find(|m| m.id == second).unwrap().pinned);
    }

    /// The four addressed feed commands, given the id that addresses nothing.
    ///
    /// `""` is what every post written before ids existed carries, so it names all of them at once
    /// and the document match cannot tell them apart. The sharp edge is `edit_status`: its author
    /// check runs over the caller's own posts while the write runs over everybody's, so Alice
    /// asking to edit "her" id-less post would have reworded whichever id-less post the document
    /// holds first; here, Bob's. Found by adversarial review.
    #[tokio::test]
    async fn a_feed_command_with_no_post_id_is_refused_rather_than_aimed_at_random() {
        let clock = ManualClock::new(T0);
        let (mut alice, bob) = status_duo(&clock).await;
        let his = bob.my_fingerprint();
        let mine = alice.my_fingerprint();

        // Two id-less posts, Bob's first, injected as if they had arrived over gossip from a
        // client that predated ids.
        alice
            .sync
            .post(DocType::Status, STATUS_DOC, move |d| {
                append_message(d, "", &his, "bob's old notice", 5, "")
            })
            .await
            .unwrap();
        alice
            .sync
            .post(DocType::Status, STATUS_DOC, move |d| {
                append_message(d, "", &mine, "alice's old notice", 6, "")
            })
            .await
            .unwrap();

        assert!(matches!(
            alice.edit_status("", "mine now").await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            alice.delete_status("").await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            alice.set_status_pin("", true).await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            alice.toggle_status_reaction("", "🎉").await,
            Err(AppError::Invalid(_))
        ));

        // Both posts are exactly as they were: the same words under the same fingerprints.
        let feed = alice.statuses();
        assert_eq!(feed.len(), 2, "nothing was deleted");
        assert_eq!(feed[0].text, "bob's old notice");
        assert_eq!(feed[0].author, bob.my_fingerprint());
        assert_eq!(feed[0].edited, 0, "and nobody's words were replaced");
        assert!(!feed[0].pinned);
        assert!(feed[0].reactions.is_empty());
        assert_eq!(feed[1].text, "alice's old notice");
    }

    #[tokio::test]
    async fn a_status_post_and_the_feeds_posting_policy_both_reach_a_joiner() {
        let clock = ManualClock::new(T0);
        let (mut alice, mut bob) = status_duo(&clock).await;
        alice.post_status("we open at eight").await.unwrap();
        alice.set_status_members_may_post(true).await.unwrap();
        drain_sync(&mut bob).await;

        let feed = bob.statuses();
        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].text, "we open at eight");
        assert_eq!(feed[0].author, alice.my_fingerprint());
        assert!(
            bob.status_members_may_post(),
            "the policy rides the feed document, so it is shared state and not a local preference"
        );
    }

    #[tokio::test]
    async fn server_events_round_trip_sorted_capped_and_author_deletable() {
        let mut alice = founder();
        alice.open_calendar().await.unwrap();
        assert!(alice.events().is_empty(), "no events by default");

        // Created out of order; `events()` returns them by start time ascending.
        let later = alice
            .create_event("Retro", "how did it go", 5_000, 6_000, "")
            .await
            .unwrap();
        let sooner = alice
            .create_event("  Standup  ", "", 2_000, 0, "")
            .await
            .unwrap();
        let feed = alice.events();
        assert_eq!(feed.len(), 2);
        assert_eq!(feed[0].id, sooner);
        assert_eq!(feed[1].id, later);
        // The title is trimmed; authorship + creation stamp come from the signed op's device
        // fingerprint and the injected clock, exactly like a status post.
        assert_eq!(feed[0].title, "Standup");
        assert_eq!(feed[0].start_ts, 2_000);
        assert_eq!(feed[0].end_ts, 0, "0 = no end time");
        assert_eq!(feed[0].author, alice.my_fingerprint());
        assert_eq!(feed[0].created_ts, 1_000);
        assert_eq!(feed[1].body, "how did it go");
        assert_eq!(feed[1].end_ts, 6_000);

        // Validation: a blank title, over-cap title/body, and an end before the start are refused.
        assert!(matches!(
            alice.create_event("   ", "b", 1, 0, "").await,
            Err(AppError::Invalid(_))
        ));
        let over_title = "t".repeat(MAX_EVENT_TITLE_BYTES + 1);
        assert!(matches!(
            alice.create_event(&over_title, "", 1, 0, "").await,
            Err(AppError::Invalid(_))
        ));
        let over_body = "b".repeat(MAX_EVENT_BODY_BYTES + 1);
        assert!(matches!(
            alice.create_event("ok", &over_body, 1, 0, "").await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            alice.create_event("backwards", "", 5_000, 4_999, "").await,
            Err(AppError::Invalid(_))
        ));
        // …but exactly at each cap is accepted, and `end_ts == start_ts` is a zero-length event.
        alice
            .create_event(
                &"t".repeat(MAX_EVENT_TITLE_BYTES),
                &"b".repeat(MAX_EVENT_BODY_BYTES),
                7_000,
                7_000,
                "",
            )
            .await
            .unwrap();
        assert_eq!(alice.events().len(), 3, "only the valid creates landed");

        // The author may delete their own event; an unknown id is refused.
        alice.delete_event(&sooner).await.unwrap();
        let ids: Vec<String> = alice.events().into_iter().map(|e| e.id).collect();
        assert!(!ids.contains(&sooner));
        assert_eq!(ids.len(), 2);
        assert!(matches!(
            alice.delete_event("no-such-event").await,
            Err(AppError::Invalid(_))
        ));
    }

    /// An event's poster image is a plain content address: normalized on the way in, refused when
    /// it isn't a hex address, and reported by `file_usage` so Properties doesn't claim the file
    /// is unused while an event is showing it.
    #[tokio::test]
    async fn event_image_is_a_validated_content_address_and_counts_as_usage() {
        let mut alice = founder();
        alice.open_calendar().await.unwrap();
        alice.open_files().await.unwrap();

        let cid = alice
            .add_file("poster.png", "image/png", "", b"png")
            .await
            .unwrap();
        let hex = cid.to_hex();
        alice
            .create_event("Launch", "", 4_000, 0, &hex.to_uppercase())
            .await
            .unwrap();
        assert_eq!(
            alice.events()[0].image,
            hex,
            "the address is stored lowercase, whatever case it arrived in"
        );

        // Shape is all that is checked, but it IS checked: a non-address never reaches the doc.
        assert!(matches!(
            alice.create_event("Bad", "", 4_000, 0, "not-a-cid").await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            alice
                .create_event("Long", "", 4_000, 0, &"a".repeat(MAX_EVENT_IMAGE_HEX + 1))
                .await,
            Err(AppError::Invalid(_))
        ));
        assert_eq!(alice.events().len(), 1, "only the valid create landed");

        // The poster is named by the `image` field, not by a body marker, so usage has to match
        // that field too; and an event body marker counts like any other reference.
        let usage = alice.file_usage(&cid);
        assert_eq!(usage.event_count, 1);
        assert!(!usage.is_empty(), "an event-only use is still a use");
        alice
            .create_event("Also", &format!("see ![p](cid:{hex})"), 5_000, 0, "")
            .await
            .unwrap();
        assert_eq!(alice.file_usage(&cid).event_count, 2);
    }

    #[tokio::test]
    async fn a_wiki_page_is_written_and_read() {
        let mut alice = founder();
        alice.open_wiki().await.unwrap();
        assert!(alice.wiki_pages().is_empty());

        alice
            .write_wiki_page("Home", "Welcome to the wiki")
            .await
            .unwrap();
        alice.write_wiki_page("Rules", "Be nice").await.unwrap();
        assert_eq!(
            alice.wiki_pages(),
            vec!["Home".to_string(), "Rules".to_string()]
        );
        assert_eq!(alice.read_wiki_page("Home"), "Welcome to the wiki");

        // Editing a page replaces its body (a sequential edit); page count unchanged.
        alice.write_wiki_page("Home", "Updated text").await.unwrap();
        assert_eq!(alice.read_wiki_page("Home"), "Updated text");
        assert_eq!(alice.wiki_pages().len(), 2);

        // A blank page name is rejected.
        assert!(matches!(
            alice.write_wiki_page("  ", "x").await,
            Err(AppError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn a_wiki_page_declares_a_render_format() {
        let mut alice = founder();
        alice.open_wiki().await.unwrap();
        alice.write_wiki_page("Home", "hello").await.unwrap();

        // Default: no metadata entry at all; the frontend reads "missing" as markdown, so a
        // doc written before formats existed keeps rendering the same way.
        assert!(alice.wiki_meta().is_empty());

        alice.set_wiki_page_format("Home", "wiki").await.unwrap();
        assert_eq!(
            alice.wiki_meta().get("Home").map(String::as_str),
            Some("wiki")
        );
        // A format is a toggle: setting it back is a plain overwrite.
        alice.set_wiki_page_format("Home", "md").await.unwrap();
        assert_eq!(
            alice.wiki_meta().get("Home").map(String::as_str),
            Some("md")
        );

        // Anything but "md"/"wiki" is refused.
        for bad in ["html", "MD", "", "markdown"] {
            assert!(
                matches!(
                    alice.set_wiki_page_format("Home", bad).await,
                    Err(AppError::Invalid(_))
                ),
                "format {bad:?} should be rejected"
            );
        }

        // The reserved metadata key is invisible to every page reader; it holds a Map, not a
        // Text, and is NUL-prefixed besides.
        assert_eq!(alice.wiki_pages(), vec!["Home".to_string()]);
        assert_eq!(alice.wiki_map().len(), 1);
        assert!(!alice.wiki_map().contains_key(WIKI_META_KEY));
        assert!(alice.read_wiki_page(WIKI_META_KEY).is_empty());
    }

    #[tokio::test]
    async fn a_reserved_or_oversize_wiki_page_name_is_rejected() {
        let mut alice = founder();
        alice.open_wiki().await.unwrap();

        let long = "x".repeat(MAX_WIKI_NAME_CHARS + 1);
        for bad in [WIKI_META_KEY, "\u{0}anything", long.as_str(), "  "] {
            assert!(
                matches!(
                    alice.write_wiki_page(bad, "body").await,
                    Err(AppError::Invalid(_))
                ),
                "name {bad:?} should be rejected"
            );
            assert!(matches!(
                alice.set_wiki_page_format(bad, "wiki").await,
                Err(AppError::Invalid(_))
            ));
        }
        // Exactly at the cap is fine; the bound is inclusive, like the frontend's grammar.
        let at_cap = "y".repeat(MAX_WIKI_NAME_CHARS);
        alice.write_wiki_page(&at_cap, "ok").await.unwrap();
        assert_eq!(alice.read_wiki_page(&at_cap), "ok");
        assert_eq!(alice.wiki_pages(), vec![at_cap]);
    }

    #[tokio::test]
    async fn a_wiki_page_is_deleted_with_its_metadata() {
        let mut alice = founder();
        alice.open_wiki().await.unwrap();
        alice.write_wiki_page("Home", "hello").await.unwrap();
        alice.write_wiki_page("Rules", "be nice").await.unwrap();
        alice.set_wiki_page_format("Home", "wiki").await.unwrap();

        alice.delete_wiki_page("Home").await.unwrap();
        assert_eq!(alice.wiki_pages(), vec!["Rules".to_string()]);
        assert!(
            !alice.wiki_meta().contains_key("Home"),
            "the metadata entry goes with the page; a recreated page starts at the default"
        );

        // Deleting a page that is not there is an error, not a silent no-op.
        assert!(matches!(
            alice.delete_wiki_page("Home").await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            alice.delete_wiki_page("never-existed").await,
            Err(AppError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn a_wiki_page_rename_carries_body_and_format() {
        let mut alice = founder();
        alice.open_wiki().await.unwrap();
        alice.write_wiki_page("Old", "the body").await.unwrap();
        alice.write_wiki_page("Other", "elsewhere").await.unwrap();
        alice.set_wiki_page_format("Old", "wiki").await.unwrap();

        alice.rename_wiki_page("Old", "New").await.unwrap();
        assert_eq!(
            alice.wiki_pages(),
            vec!["New".to_string(), "Other".to_string()]
        );
        assert_eq!(alice.read_wiki_page("New"), "the body");
        assert_eq!(
            alice.wiki_meta().get("New").map(String::as_str),
            Some("wiki")
        );
        assert!(!alice.wiki_meta().contains_key("Old"));

        // A rename never clobbers an existing page, and a missing source is an error.
        alice.write_wiki_page("Taken", "keep me").await.unwrap();
        assert!(matches!(
            alice.rename_wiki_page("New", "Taken").await,
            Err(AppError::Invalid(_))
        ));
        assert_eq!(alice.read_wiki_page("Taken"), "keep me");
        assert!(matches!(
            alice.rename_wiki_page("Old", "Elsewhere").await,
            Err(AppError::Invalid(_))
        ));
        // The destination is validated like any new page name.
        assert!(matches!(
            alice
                .rename_wiki_page("New", &"z".repeat(MAX_WIKI_NAME_CHARS + 1))
                .await,
            Err(AppError::Invalid(_))
        ));
    }

    #[test]
    fn wiki_formats_on_different_pages_both_survive_a_merge() {
        // Two members concurrently declare a format on DIFFERENT pages. The metadata map is a
        // single container created by whoever gets there first, so this is the case that would
        // lose a write if each fork put a *fresh* Map at the reserved key.
        let mut base = AutoCommit::new();
        write_wiki_page(&mut base, "A", "a").unwrap();
        write_wiki_page(&mut base, "B", "b").unwrap();
        let mut a = base.fork();
        let mut b = base.fork();
        set_wiki_format(&mut a, "A", "wiki").unwrap();
        set_wiki_format(&mut b, "B", "md").unwrap();
        a.merge(&mut b).unwrap();

        let meta = read_wiki_meta(&a);
        assert_eq!(meta.get("A").map(String::as_str), Some("wiki"));
        assert_eq!(meta.get("B").map(String::as_str), Some("md"));
        // …and the metadata still never shows up as a page.
        let mut pages: Vec<String> = read_wiki_map(&a).into_keys().collect();
        pages.sort();
        assert_eq!(pages, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn wiki_page_bodies_merge_character_by_character() {
        // Two members concurrently edit the SAME page in non-overlapping places; both
        // edits survive on convergence (a real CRDT merge, not last-writer-wins).
        let mut base = AutoCommit::new();
        write_wiki_page(&mut base, "P", "Hello world").unwrap();
        let mut a = base.fork();
        let mut b = base.fork();
        write_wiki_page(&mut a, "P", "Hello dear world").unwrap(); // inserted "dear "
        write_wiki_page(&mut b, "P", "Hello world!").unwrap(); // appended "!"
        a.merge(&mut b).unwrap();
        assert_eq!(
            read_wiki_map(&a).get("P").map(String::as_str),
            Some("Hello dear world!")
        );
    }

    // ---------- wiki history + review mode (11x) ----------

    /// Found Alice and join Bob over one hub, both with the SAME manual clock, wikis open.
    async fn wiki_duo(
        clock: &ManualClock,
    ) -> (
        Server<MemNetwork, ChaCha20Rng>,
        Server<MemNetwork, ChaCha20Rng>,
        PeerId,
    ) {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice = Server::found(
            hub.join(alice_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(clock.clone()),
            "alice",
        )
        .unwrap();
        alice.subscribe_control().await.unwrap();
        let invite = alice.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (bob, _) = tokio::join!(
            Server::join(
                hub.join(PeerId::from_u64(2)),
                MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(2),
                Box::new(clock.clone()),
                "bob",
                alice_peer,
                &invite,
            ),
            alice.sync_once(),
        );
        let mut bob = bob.unwrap();
        alice.open_wiki().await.unwrap();
        bob.open_wiki().await.unwrap();
        (alice, bob, alice_peer)
    }

    #[tokio::test]
    async fn wiki_history_records_edits_and_a_rollback_restores() {
        let clock = ManualClock::new(T0);
        let hub = Hub::new();
        let mut alice = founder_on(&hub, PeerId::from_u64(1), &clock, 1);
        alice.open_wiki().await.unwrap();
        assert!(
            !alice.write_wiki_page("Home", "first").await.unwrap(),
            "review off: the write publishes"
        );
        clock.advance_ms(1_000);
        alice.write_wiki_page("Home", "second").await.unwrap();

        let revs = alice.wiki_history("Home");
        assert_eq!(revs.len(), 2);
        assert!(revs.iter().all(|r| r.kind == "edit"));
        assert!(revs.iter().all(|r| r.author == alice.my_fingerprint()));
        assert_eq!(revs[0].body, "first");
        assert_eq!(revs[1].body, "second");
        assert!(revs[0].ts <= revs[1].ts);
        assert!(alice.wiki_history("Nowhere").is_empty());

        // Roll back to the first revision: the body returns and the act is itself a revision.
        clock.advance_ms(1_000);
        let queued = alice
            .restore_wiki_page("Home", &revs[0].id.clone())
            .await
            .unwrap();
        assert!(!queued);
        assert_eq!(alice.read_wiki_page("Home"), "first");
        let revs2 = alice.wiki_history("Home");
        assert_eq!(revs2.len(), 3);
        assert_eq!(revs2[2].kind, "rollback");
        assert_eq!(revs2[2].note, revs[0].id);
        assert!(matches!(
            alice.restore_wiki_page("Home", "no-such-rev").await,
            Err(AppError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn the_wiki_review_window_is_role_gated_and_capped() {
        let clock = ManualClock::new(T0);
        let (mut alice, mut bob, _) = wiki_duo(&clock).await;
        assert_eq!(alice.wiki_review_days(), 0, "review starts off");

        assert!(matches!(
            bob.set_wiki_review_days(5).await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            alice.set_wiki_review_days(MAX_WIKI_REVIEW_DAYS + 1).await,
            Err(AppError::Invalid(_))
        ));
        alice.set_wiki_review_days(5).await.unwrap();
        assert_eq!(alice.wiki_review_days(), 5);
        // The setting is shared state: Bob reads it once the op reaches him.
        bob.sync_once().await.unwrap();
        assert_eq!(bob.wiki_review_days(), 5);
    }

    #[tokio::test]
    async fn a_member_edit_queues_and_an_admin_approves_or_declines() {
        let clock = ManualClock::new(T0);
        let (mut alice, mut bob, _) = wiki_duo(&clock).await;
        alice.set_wiki_review_days(5).await.unwrap();
        bob.sync_once().await.unwrap();

        // Bob (a plain member) saves: queued, not published; the owner publishes directly.
        assert!(bob.write_wiki_page("Rules", "be nice").await.unwrap());
        assert!(!bob.wiki_map().contains_key("Rules"), "not live yet");
        alice.sync_once().await.unwrap();
        let queue = alice.wiki_pending_edits();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].page, "Rules");
        assert_eq!(queue[0].author, bob.my_fingerprint());
        assert_eq!(queue[0].expires_ts, T0 + 5 * DAY_MS);
        assert!(
            !alice.write_wiki_page("Home", "hi").await.unwrap(),
            "owner edits bypass review"
        );

        // Bob may not approve; Alice approves and the page goes live with a history record.
        assert!(matches!(
            bob.approve_wiki_edit(&queue[0].id).await,
            Err(AppError::Invalid(_))
        ));
        alice.approve_wiki_edit(&queue[0].id).await.unwrap();
        assert_eq!(alice.read_wiki_page("Rules"), "be nice");
        assert!(alice.wiki_pending_edits().is_empty());
        let revs = alice.wiki_history("Rules");
        assert_eq!(revs.len(), 1);
        assert_eq!(revs[0].kind, "approve");
        assert_eq!(revs[0].author, bob.my_fingerprint());
        assert_eq!(revs[0].actor, alice.my_fingerprint());
        assert_eq!(revs[0].id, queue[0].id);
        drain_sync(&mut bob).await;
        assert_eq!(bob.read_wiki_page("Rules"), "be nice");

        // A second proposal is declined: never live, dropped from the queue, auditable.
        clock.advance_ms(1_000);
        assert!(bob.write_wiki_page("Rules", "no rules!").await.unwrap());
        alice.sync_once().await.unwrap();
        let queue = alice.wiki_pending_edits();
        assert_eq!(queue.len(), 1);
        alice.reject_wiki_edit(&queue[0].id).await.unwrap();
        assert_eq!(alice.read_wiki_page("Rules"), "be nice");
        assert!(alice.wiki_pending_edits().is_empty());
        let revs = alice.wiki_history("Rules");
        assert_eq!(revs.len(), 2);
        assert_eq!(revs[1].kind, "reject");
        assert_eq!(revs[1].body, "no rules!");
        assert!(matches!(
            alice.approve_wiki_edit("no-such-pending").await,
            Err(AppError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn an_unreviewed_edit_auto_accepts_at_its_deadline() {
        let clock = ManualClock::new(T0);
        let (mut alice, mut bob, _) = wiki_duo(&clock).await;
        alice.set_wiki_review_days(3).await.unwrap();
        bob.sync_once().await.unwrap();

        assert!(bob.write_wiki_page("Lore", "the tale").await.unwrap());
        alice.sync_once().await.unwrap();
        assert!(!alice.wiki_map().contains_key("Lore"));

        // Nobody reviews it; at the deadline every reader sees it accepted, deterministically.
        clock.advance_ms(3 * DAY_MS);
        assert_eq!(alice.read_wiki_page("Lore"), "the tale");
        assert_eq!(bob.read_wiki_page("Lore"), "the tale");
        assert!(
            alice.wiki_pending_edits().is_empty(),
            "past its window it is no longer reviewable"
        );
        let revs = alice.wiki_history("Lore");
        assert_eq!(revs.len(), 1);
        assert_eq!(revs[0].kind, "auto");
        assert_eq!(revs[0].author, bob.my_fingerprint());
        assert_eq!(revs[0].ts, T0 + 3 * DAY_MS);
        // Declining now is too late.
        assert!(matches!(
            alice.reject_wiki_edit(&revs[0].id).await,
            Err(AppError::Invalid(_))
        ));

        // The next direct write folds the acceptance into stored history and supersedes it.
        clock.advance_ms(1_000);
        alice
            .write_wiki_page("Lore", "the tale, edited")
            .await
            .unwrap();
        let revs = alice.wiki_history("Lore");
        assert_eq!(revs.len(), 2);
        assert_eq!(revs[0].kind, "auto");
        assert_eq!(revs[1].kind, "edit");
        assert_eq!(alice.read_wiki_page("Lore"), "the tale, edited");
        bob.sync_once().await.unwrap();
        assert_eq!(bob.read_wiki_page("Lore"), "the tale, edited");
        assert_eq!(bob.wiki_history("Lore").len(), 2);
    }

    #[tokio::test]
    async fn approving_a_proposal_settles_an_older_one_that_lapsed_while_it_waited() {
        // Two proposals for one page. The first lapses (auto-accepted by the read-time overlay)
        // while the second is still in review. Approving the second must WIN: if the lapsed one
        // stayed in the queue it would keep overlaying its body on top of the approval.
        let clock = ManualClock::new(T0);
        let (mut alice, mut bob, _) = wiki_duo(&clock).await;
        alice.set_wiki_review_days(2).await.unwrap();
        bob.sync_once().await.unwrap();

        assert!(bob.write_wiki_page("Lore", "first draft").await.unwrap());
        clock.advance_ms(DAY_MS);
        assert!(bob.write_wiki_page("Lore", "second draft").await.unwrap());
        // Two proposals were posted, so one tick would apply only the first.
        drain_sync(&mut alice).await;
        assert_eq!(alice.wiki_pending_edits().len(), 2);

        // Day 2 + a moment: the first proposal's window has lapsed, the second's has not.
        clock.advance_ms(DAY_MS + 1);
        let queue = alice.wiki_pending_edits();
        assert_eq!(queue.len(), 1, "only the second is still reviewable");
        assert_eq!(
            alice.read_wiki_page("Lore"),
            "first draft",
            "the lapsed one is live"
        );

        alice.approve_wiki_edit(&queue[0].id).await.unwrap();
        assert_eq!(
            alice.read_wiki_page("Lore"),
            "second draft",
            "the approval is the live body; the lapsed proposal must not override it"
        );
        assert!(alice.wiki_pending_edits().is_empty());
        drain_sync(&mut bob).await;
        assert_eq!(bob.read_wiki_page("Lore"), "second draft");

        // Both are recorded, in the order they took effect.
        let revs = alice.wiki_history("Lore");
        assert_eq!(revs.len(), 2);
        assert_eq!(revs[0].kind, "auto");
        assert_eq!(revs[0].body, "first draft");
        assert_eq!(revs[1].kind, "approve");
        assert_eq!(revs[1].body, "second draft");
    }

    #[tokio::test]
    async fn review_mode_gates_member_delete_and_rename() {
        let clock = ManualClock::new(T0);
        let (mut alice, mut bob, _) = wiki_duo(&clock).await;
        alice
            .write_wiki_page("Guides/Setup", "plug it in")
            .await
            .unwrap();
        bob.sync_once().await.unwrap();

        // Review off: a member deletes/renames like today.
        clock.advance_ms(1_000);
        bob.rename_wiki_page("Guides/Setup", "Guides/Install")
            .await
            .unwrap();
        alice.sync_once().await.unwrap();
        assert_eq!(alice.read_wiki_page("Guides/Install"), "plug it in");
        let revs = alice.wiki_history("Guides/Install");
        assert_eq!(revs.last().unwrap().kind, "rename");
        assert_eq!(revs.last().unwrap().note, "Guides/Setup");

        // Review on: a member's delete/rename would sidestep the queue, so both are gated.
        alice.set_wiki_review_days(7).await.unwrap();
        bob.sync_once().await.unwrap();
        assert!(matches!(
            bob.delete_wiki_page("Guides/Install").await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            bob.rename_wiki_page("Guides/Install", "Elsewhere").await,
            Err(AppError::Invalid(_))
        ));

        // The owner's delete records the last body and drops open proposals for the page.
        assert!(bob
            .write_wiki_page("Guides/Install", "unplug it")
            .await
            .unwrap());
        alice.sync_once().await.unwrap();
        assert_eq!(alice.wiki_pending_edits().len(), 1);
        clock.advance_ms(1_000);
        alice.delete_wiki_page("Guides/Install").await.unwrap();
        assert!(!alice.wiki_map().contains_key("Guides/Install"));
        assert!(alice.wiki_pending_edits().is_empty());
        let revs = alice.wiki_history("Guides/Install");
        assert_eq!(revs.last().unwrap().kind, "delete");
        assert_eq!(revs.last().unwrap().body, "plug it in");
        // With the page (and its proposals) gone, the deadline passing resurrects nothing.
        clock.advance_ms(8 * DAY_MS);
        assert!(!alice.wiki_map().contains_key("Guides/Install"));
    }

    #[test]
    fn concurrent_folds_of_one_expired_pending_converge_to_one_revision() {
        // Two peers both fold the same expired pending edit into history (each alongside its
        // own direct write), then merge: the deterministic revision id (= the pending id)
        // keys ONE history entry, and the queue entry is gone everywhere.
        let mut base = AutoCommit::new();
        write_wiki_page(&mut base, "P", "old").unwrap();
        write_wiki_pending(
            &mut base,
            &WikiPendingEdit {
                id: "pend-1".into(),
                page: "P".into(),
                author: "carol".into(),
                ts: 1_000,
                expires_ts: 2_000,
                body: "proposed".into(),
            },
        )
        .unwrap();
        let mut a = base.fork();
        let mut b = base.fork();
        fold_expired_wiki_pending(&mut a, "P", 3_000).unwrap();
        write_wiki_page(&mut a, "P", "a's edit").unwrap();
        fold_expired_wiki_pending(&mut b, "P", 3_000).unwrap();
        write_wiki_page(&mut b, "P", "b's edit").unwrap();
        a.merge(&mut b).unwrap();

        assert!(read_wiki_pending_all(&a).is_empty());
        let revs = read_wiki_history(&a, "P");
        assert_eq!(revs.len(), 1, "one converged auto revision, not two");
        assert_eq!(revs[0].id, "pend-1");
        assert_eq!(revs[0].kind, "auto");
        assert_eq!(revs[0].body, "proposed");
    }

    #[tokio::test]
    async fn a_pending_embed_pins_its_file() {
        let clock = ManualClock::new(T0);
        let (mut alice, mut bob, _) = wiki_duo(&clock).await;
        alice.open_files().await.unwrap();
        bob.open_files().await.unwrap();
        alice.set_wiki_review_days(5).await.unwrap();
        bob.sync_once().await.unwrap();

        let cid = bob
            .add_file("cat.png", "image/png", "wiki/Cats", b"CAT")
            .await
            .unwrap();
        assert!(bob
            .write_wiki_page("Cats", &format!("![a cat](cid:{})", cid.to_hex()))
            .await
            .unwrap());
        // Queued, not live; but the embed must survive until the edit lands either way.
        assert!(!bob.wiki_map().contains_key("Cats"));
        assert!(bob.wiki_pinned_cids().contains(&cid.to_hex()));
    }

    #[tokio::test]
    async fn an_oversize_file_is_rejected() {
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let big = vec![0u8; MAX_FILE_BYTES + 1];
        assert!(matches!(
            alice
                .add_file("big.bin", "application/octet-stream", "", &big)
                .await,
            Err(AppError::Invalid(_))
        ));
    }

    #[test]
    fn the_chunk_size_is_the_padding_ladders_ceiling() {
        // Load-bearing equality (P10). If the chunk size drifts above the ladder ceiling, every
        // full chunk stops being padded and short tails stop hiding among them; if it drifts
        // below, every full chunk pays a whole bucket step and the product's bulk traffic
        // roughly doubles.
        assert_eq!(CHUNK_BYTES, catcoms_storage::CHUNK_PAD_CEILING);
        assert!(CHUNK_BYTES.is_power_of_two());
    }

    #[tokio::test]
    async fn a_small_shared_file_is_stored_at_its_bucket_size() {
        // End to end at the product layer: what a peer fetches for a small file is the ladder
        // bucket, not the file size. A 200 KB image and a 150 KB image are the same fetch.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let mut sizes = Vec::new();
        for (i, n) in [150_000usize, 200_000, 300_000].into_iter().enumerate() {
            let data: Vec<u8> = (0..n).map(|k| ((k + i) % 251) as u8).collect();
            let cid = alice
                .add_file(&format!("img{i}.bin"), "image/png", "", &data)
                .await
                .unwrap();
            let entry = alice
                .files()
                .into_iter()
                .find(|e| e.cid == cid.as_bytes().to_vec())
                .expect("listed");
            let manifest = FileManifest::decode_or_legacy(&entry.file_ref).unwrap();
            assert_eq!(manifest.chunks.len(), 1, "a sub-chunk file is one blob");
            let blob = alice
                .sync
                .get_blob(&manifest.chunks[0].ciphertext_cid)
                .expect("held locally");
            // The declared size still describes the true plaintext, so the UI and the whole-file
            // address are unaffected.
            assert_eq!(entry.size, n as u64);
            sizes.push(blob.len());
            // ...and the file still reassembles to exactly the original bytes.
            assert_eq!(alice.download_file(&cid).await.unwrap(), data);
        }
        assert_eq!(
            sizes[0], sizes[1],
            "150 KB and 200 KB must be indistinguishable on the wire"
        );
        assert_ne!(sizes[1], sizes[2], "the next bucket up is a distinct class");
        assert_eq!(sizes[0], 256 * 1024 + 24 + 4 + 16);
    }

    #[tokio::test]
    async fn a_media_range_reads_exactly_the_requested_window_across_a_chunk_boundary() {
        // The media path's whole reason to exist: serve the window a player asked for without
        // reading everything before it. The pattern is position-dependent, so a range that came
        // back off by even one byte (bad chunk index, bad intra-chunk offset) fails loudly.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let n = CHUNK_BYTES * 2 + 4242;
        let data: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
        let cid = alice
            .add_file("clip.bin", "video/mp4", "", &data)
            .await
            .unwrap();
        let manifest_version = alice.file_head(&cid).unwrap().manifest_version;

        // A window wholly inside the first chunk.
        let head = alice
            .read_file_range(&cid, manifest_version, 0, 1024)
            .await
            .unwrap();
        assert_eq!(head.bytes, data[..1024]);
        assert_eq!(head.total_size, n as u64);
        assert_eq!(head.mime, "video/mp4");
        assert!(
            head.provider.is_none(),
            "the uploader holds every chunk, so nothing may be requested from the network"
        );

        // A window straddling the first/second chunk boundary: the case plain chunk indexing
        // gets wrong.
        let start = CHUNK_BYTES as u64 - 10;
        let span = alice
            .read_file_range(&cid, manifest_version, start, 20)
            .await
            .unwrap();
        assert_eq!(span.bytes, data[start as usize..start as usize + 20]);

        // A window in the short tail chunk, clamped to the end of the file.
        let tail_start = (CHUNK_BYTES * 2) as u64 + 4000;
        let tail = alice
            .read_file_range(&cid, manifest_version, tail_start, 8192)
            .await
            .unwrap();
        assert_eq!(tail.bytes, data[tail_start as usize..]);
        assert_eq!(tail.bytes.len(), 242, "the read stops at the end of file");

        // Seeking past the end is a normal thing for a player to probe; it is not an error.
        let past = alice
            .read_file_range(&cid, manifest_version, n as u64 + 99, 512)
            .await
            .unwrap();
        assert!(past.bytes.is_empty());
        assert_eq!(past.total_size, n as u64);
    }

    #[tokio::test]
    async fn media_reads_reject_ambiguous_and_stale_manifest_versions() {
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let safe = b"\x89PNG\r\n\x1a\nsafe image";
        let replacement = b"ID3replacement audio";
        let cid = alice
            .add_file("safe.png", "image/png", "safe", safe)
            .await
            .unwrap();
        let old_version = alice.file_head(&cid).unwrap().manifest_version;
        let donor_cid = alice
            .add_file("donor.mp3", "audio/mpeg", "donor", replacement)
            .await
            .unwrap();
        let donor = alice
            .files()
            .into_iter()
            .find(|entry| entry.cid == donor_cid.as_bytes().to_vec())
            .unwrap();
        let mut forged = FileManifest::decode_or_legacy(&donor.file_ref).unwrap();
        forged.plaintext_cid = cid;
        let forged_ref = forged.encode();
        let author = alice.my_fingerprint();
        let (public_key, signature) = alice
            .file_entry_attestation("replacement.mp3", &author, "forged", &forged_ref)
            .unwrap();
        alice
            .sync
            .post(DocType::FileIndex, FILE_INDEX_DOC, |doc| {
                write_file_entry(
                    doc,
                    "replacement.mp3",
                    &author,
                    "forged",
                    &forged_ref,
                    FileExpiry::Never,
                    Some((&public_key, &signature)),
                )
            })
            .await
            .unwrap();

        assert!(
            alice.file_head(&cid).is_none(),
            "the same claimed plaintext CID with two manifests must be denied as ambiguous"
        );

        alice.delete_file_at(&cid, "safe").await.unwrap();
        let replacement_head = alice.file_head(&cid).expect("forged row remains current");
        assert_ne!(replacement_head.manifest_version, old_version);
        assert!(
            alice
                .read_file_range(&cid, old_version, 0, 64)
                .await
                .is_err(),
            "a MIME decision captured for the removed manifest cannot authorize replacement bytes"
        );
        assert_eq!(
            alice
                .read_file_range(&cid, replacement_head.manifest_version, 0, 64)
                .await
                .unwrap()
                .bytes,
            replacement
        );
    }

    #[tokio::test]
    async fn a_media_range_refuses_a_file_that_is_not_listed() {
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let missing = Cid::of(b"nothing here");
        assert!(alice
            .read_file_range(&missing, [0; 32], 0, 16)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_large_file_is_chunked_and_reassembles() {
        // A file larger than one chunk is split into multiple sealed chunks + a manifest, and
        // download reassembles it byte-for-byte. Use a position-dependent pattern so a mis-ordered
        // or dropped chunk would change the result.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let n = CHUNK_BYTES + 1234;
        let data: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
        let cid = alice
            .add_file("big.bin", "application/octet-stream", "", &data)
            .await
            .unwrap();

        // The manifest split it into more than one chunk, each under the per-blob cap.
        let entry = alice
            .files()
            .into_iter()
            .find(|e| e.cid == cid.as_bytes().to_vec())
            .expect("the file is listed");
        let manifest = FileManifest::decode_or_legacy(&entry.file_ref).unwrap();
        assert!(
            manifest.chunks.len() >= 2,
            "a larger-than-chunk file is split into multiple chunks"
        );
        assert_eq!(manifest.total_size, n as u64);
        assert_eq!(
            cid,
            Cid::of(&data),
            "the identity is the whole-file plaintext cid"
        );

        // It reassembles to the exact original (the uploader holds every chunk locally).
        assert!(alice.file_available(&cid), "all chunks are held locally");
        let got = alice.download_file(&cid).await.unwrap();
        assert_eq!(got, data, "the chunked file reassembles byte-for-byte");
    }

    #[tokio::test]
    async fn upload_progress_finishes_only_after_the_file_is_published() {
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let data = vec![7u8; CHUNK_BYTES + 1];
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);

        alice
            .add_file_with_progress(
                "progress.bin",
                "application/octet-stream",
                "",
                &data,
                Some(&tx),
            )
            .await
            .unwrap();

        assert_eq!(rx.recv().await, Some((0, 3)));
        assert_eq!(rx.recv().await, Some((1, 3)));
        assert_eq!(rx.recv().await, Some((2, 3)));
        assert_eq!(rx.recv().await, Some((3, 3)));
        assert_eq!(alice.files().len(), 1, "100% means the listing is visible");
    }

    /// Push `data` through the streaming upload path one chunk at a time, exactly as the desktop
    /// bridge does: seal each `CHUNK_BYTES` slice on its own, accumulate the whole-file address,
    /// then publish the manifest.
    async fn stream_upload(
        server: &mut Server<MemNetwork, ChaCha20Rng>,
        name: &str,
        path: &str,
        data: &[u8],
    ) -> Cid {
        let mime = "application/octet-stream";
        let mut address = catcoms_storage::CidHasher::new();
        let mut chunks = Vec::new();
        for chunk in data.chunks(CHUNK_BYTES) {
            address.update(chunk);
            chunks.push(server.seal_upload_chunk(chunk, mime).unwrap());
        }
        if chunks.is_empty() {
            chunks.push(server.seal_upload_chunk(&[], mime).unwrap());
        }
        server
            .publish_upload(name, mime, path, address.cid(), data.len() as u64, chunks)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_streamed_upload_is_identical_to_a_whole_file_one() {
        // The bridge streams a file chunk by chunk so neither the webview nor the actor is ever
        // occupied for a whole transfer. The result must be indistinguishable from add_file's:
        // same identity, same chunk boundaries, and it reassembles byte-for-byte.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let n = CHUNK_BYTES * 2 + 77;
        let data: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();

        let cid = stream_upload(&mut alice, "streamed.bin", "", &data).await;

        assert_eq!(
            cid,
            Cid::of(&data),
            "the identity is the whole-file address"
        );
        let entry = alice
            .files()
            .into_iter()
            .find(|e| e.cid == cid.as_bytes().to_vec())
            .expect("the streamed file is listed");
        let manifest = FileManifest::decode_or_legacy(&entry.file_ref).unwrap();
        assert_eq!(manifest.chunks.len(), 3, "one chunk per CHUNK_BYTES slice");
        assert_eq!(manifest.total_size, n as u64);
        assert!(alice.file_available(&cid), "every chunk is held locally");
        assert_eq!(alice.download_file(&cid).await.unwrap(), data);
    }

    #[tokio::test]
    async fn an_empty_streamed_upload_still_publishes_one_chunk() {
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let cid = stream_upload(&mut alice, "empty.bin", "", &[]).await;
        assert_eq!(cid, Cid::of(b""));
        assert_eq!(alice.download_file(&cid).await.unwrap(), Vec::<u8>::new());
    }

    #[tokio::test]
    async fn a_streamed_upload_of_already_shared_content_dedups_and_drops_its_chunks() {
        // A streamed upload only learns its whole-file address after the last chunk, so its dedup
        // check lands after sealing. The second listing must reuse the first's ref and the blobs
        // the second upload sealed must be gone: dedup that left a second sealed copy behind
        // would defeat the point of dedup.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let data: Vec<u8> = (0..CHUNK_BYTES + 9).map(|i| (i % 251) as u8).collect();

        let first = stream_upload(&mut alice, "twin.bin", "", &data).await;
        let after_first = alice.sync().blob_cids().len();

        let second = stream_upload(&mut alice, "twin.bin", "elsewhere", &data).await;

        assert_eq!(second, first, "identical content keeps one identity");
        assert_eq!(
            alice.sync().blob_cids().len(),
            after_first,
            "the redundant sealed chunks were collected"
        );
        let listings: Vec<FileEntry> = alice
            .files()
            .into_iter()
            .filter(|e| e.cid == first.as_bytes().to_vec())
            .collect();
        assert_eq!(listings.len(), 2, "both folders list the content");
        assert_eq!(
            listings[0].file_ref, listings[1].file_ref,
            "the second listing names the first upload's blobs"
        );
        assert_eq!(alice.download_file(&first).await.unwrap(), data);
    }

    #[tokio::test]
    async fn re_streaming_the_same_name_and_folder_changes_nothing() {
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let data = vec![3u8; 4096];
        let first = stream_upload(&mut alice, "same.bin", "docs", &data).await;
        let blobs = alice.sync().blob_cids().len();

        let again = stream_upload(&mut alice, "same.bin", "docs", &data).await;

        assert_eq!(again, first);
        assert_eq!(alice.files().len(), 1, "no duplicate listing");
        assert_eq!(alice.sync().blob_cids().len(), blobs, "nothing new sealed");
    }

    #[tokio::test]
    async fn a_sealed_chunk_is_not_held_content_until_its_upload_publishes() {
        // The distinction staging exists to make. A chunk on disk that no manifest names must not
        // look like a chunk this device holds: that is what let an interrupted upload leave blobs
        // indistinguishable from real ones, which no sweep could then safely reclaim.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let staged = alice
            .seal_upload_chunk(b"in flight", "application/octet-stream")
            .unwrap();
        assert!(
            !alice.sync().has_blob(&staged.ciphertext_cid),
            "staged, so not held"
        );
        assert!(
            !alice.sync().blob_cids().contains(&staged.ciphertext_cid),
            "and not in the store's inventory"
        );

        // Publishing is what makes it real.
        let data = b"in flight".to_vec();
        alice
            .publish_upload(
                "f.bin",
                "application/octet-stream",
                "",
                Cid::of(&data),
                data.len() as u64,
                vec![staged.clone()],
            )
            .await
            .unwrap();
        assert!(alice.sync().has_blob(&staged.ciphertext_cid), "now held");
        assert_eq!(alice.download_file(&Cid::of(&data)).await.unwrap(), data);
    }

    #[tokio::test]
    async fn a_restart_drops_the_chunks_of_uploads_that_never_finished() {
        // The crash window an in-memory record of pending uploads could not close: the process
        // that knew what these chunks were for is gone, so nothing can ever claim them.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let published = stream_upload(&mut alice, "kept.bin", "", &vec![5u8; 2048]).await;
        let held_before = alice.sync().blob_cids().len();
        alice
            .seal_upload_chunk(b"upload interrupted here", "application/octet-stream")
            .unwrap();
        alice
            .seal_upload_chunk(b"and here", "application/octet-stream")
            .unwrap();

        assert_eq!(alice.clear_staged_uploads(), 2, "both staged chunks went");

        assert_eq!(
            alice.sync().blob_cids().len(),
            held_before,
            "and the sweep touched nothing that was published"
        );
        assert_eq!(
            alice.download_file(&published).await.unwrap(),
            vec![5u8; 2048]
        );
        assert_eq!(alice.clear_staged_uploads(), 0, "a second sweep is a no-op");
    }

    #[tokio::test]
    async fn discarding_an_abandoned_upload_keeps_chunks_a_listed_file_still_needs() {
        // Cancelling an upload collects what it sealed, and cannot take storage a shared file
        // depends on: an unpublished chunk is staged, which nothing references by construction.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let data = vec![5u8; 2048];
        let cid = stream_upload(&mut alice, "kept.bin", "", &data).await;
        let listed = alice
            .files()
            .into_iter()
            .find(|e| e.cid == cid.as_bytes().to_vec())
            .unwrap();
        let live = FileManifest::decode_or_legacy(&listed.file_ref)
            .unwrap()
            .chunks;

        // An upload that got one chunk in and was then abandoned.
        let orphan = alice
            .seal_upload_chunk(b"never published", "application/octet-stream")
            .unwrap();

        let mut both = live.clone();
        both.push(orphan.clone());
        alice.discard_upload_chunks(&both);

        assert!(
            !alice.sync().has_blob(&orphan.ciphertext_cid),
            "the abandoned chunk was collected"
        );
        for chunk in &live {
            assert!(
                alice.sync().has_blob(&chunk.ciphertext_cid),
                "a chunk the listed file references is kept"
            );
        }
        assert_eq!(alice.download_file(&cid).await.unwrap(), data);
    }

    #[tokio::test]
    async fn a_streamed_upload_refuses_an_oversized_chunk() {
        // Chunks are uniform by construction (the media reader divides a byte offset by
        // CHUNK_BYTES to get an index), so an over-long chunk is rejected rather than sealed.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let too_big = vec![0u8; CHUNK_BYTES + 1];
        assert!(alice
            .seal_upload_chunk(&too_big, "application/octet-stream")
            .is_err());
    }

    #[tokio::test]
    async fn the_per_chunk_download_path_reassembles_byte_for_byte() {
        // The non-blocking download fetches ONE chunk per command (file_download_plan +
        // fetch_file_chunk) so the actor stays responsive; the per-chunk path must reassemble
        // identically to the all-in-one download.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let n = CHUNK_BYTES * 2 + 77;
        let data: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
        let cid = alice
            .add_file("big.bin", "application/octet-stream", "", &data)
            .await
            .unwrap();

        let (total, size) = alice
            .file_download_plan(&cid)
            .expect("a plan for a listed file");
        assert!(total >= 3, "split into multiple chunks");
        assert_eq!(size, n as u64);

        let mut out = Vec::with_capacity(size as usize);
        for i in 0..total {
            let (chunk, _provider) = alice.fetch_file_chunk(&cid, i).await.unwrap();
            out.extend_from_slice(&chunk);
        }
        assert_eq!(out, data, "the per-chunk fetch reassembles byte-for-byte");
        assert_eq!(Cid::of(&out), cid, "and verifies to the whole-file cid");

        // No plan for an unknown file; an out-of-range chunk errors.
        assert!(alice.file_download_plan(&Cid::of(b"nope")).is_none());
        assert!(
            alice.fetch_file_chunk(&cid, total).await.is_err(),
            "an out-of-range chunk index errors",
        );
    }

    #[tokio::test]
    async fn an_empty_file_round_trips() {
        // An empty file is one empty chunk; it still uploads, lists, and downloads back to empty.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let cid = alice
            .add_file("empty.bin", "application/octet-stream", "", b"")
            .await
            .unwrap();
        assert_eq!(cid, Cid::of(b""));
        let got = alice.download_file(&cid).await.unwrap();
        assert!(got.is_empty(), "an empty file downloads back to empty");
    }

    #[tokio::test]
    async fn deleting_a_file_garbage_collects_its_orphaned_chunk_blobs() {
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let cid_a = alice
            .add_file(
                "a.bin",
                "application/octet-stream",
                "",
                b"the bytes of file A",
            )
            .await
            .unwrap();
        let cid_b = alice
            .add_file(
                "b.bin",
                "application/octet-stream",
                "",
                b"the bytes of file B",
            )
            .await
            .unwrap();
        // Capture A's chunk ciphertext cids before deleting.
        let a_chunks: Vec<Cid> = alice
            .files()
            .iter()
            .filter(|e| e.cid == cid_a.as_bytes().to_vec())
            .filter_map(|e| FileManifest::decode_or_legacy(&e.file_ref).ok())
            .flat_map(|m| m.chunks.into_iter().map(|c| c.ciphertext_cid))
            .collect();
        assert!(!a_chunks.is_empty());
        assert!(
            a_chunks.iter().all(|c| alice.sync().has_blob(c)),
            "A's chunks are held"
        );

        alice.delete_file(&cid_a).await.unwrap();

        // A's chunk blobs are GC'd (no longer held); B is untouched + still downloadable.
        for c in &a_chunks {
            assert!(
                !alice.sync().has_blob(c),
                "A's orphaned chunk blob was deleted"
            );
        }
        assert!(
            alice.file_available(&cid_b),
            "the other file's chunks survive"
        );
        assert_eq!(
            alice.download_file(&cid_b).await.unwrap(),
            b"the bytes of file B".to_vec()
        );
    }

    // ---------- content-hash dedup on upload ----------

    /// The chunk ciphertext addresses a listed file's manifest names (empty if unlisted).
    fn chunk_cids(server: &Server<MemNetwork, ChaCha20Rng>, cid: &Cid, path: &str) -> Vec<Cid> {
        server
            .files()
            .iter()
            .filter(|e| e.cid.as_slice() == cid.as_bytes() && e.path == path)
            .filter_map(|e| FileManifest::decode_or_legacy(&e.file_ref).ok())
            .flat_map(|m| m.chunks.into_iter().map(|c| c.ciphertext_cid))
            .collect()
    }

    #[tokio::test]
    async fn re_sharing_the_same_bytes_under_the_same_name_and_folder_adds_nothing() {
        // The idempotent re-share (a double-submit, or the same asset re-attached to the page it
        // is already on): one entry, the same address back, and not a byte more stored.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let data = b"the bytes of a cat picture";
        let cid = alice
            .add_file("cat.png", "image/png", "wiki/Cats", data)
            .await
            .unwrap();
        let blobs = alice.sync().blob_cids().len();

        let again = alice
            .add_file("cat.png", "image/png", "wiki/Cats", data)
            .await
            .unwrap();
        assert_eq!(again, cid, "the second add returns the same address");
        assert_eq!(alice.files().len(), 1, "no second index entry");
        assert_eq!(
            alice.sync().blob_cids().len(),
            blobs,
            "nothing new was stored"
        );

        // The folder is compared *normalized*, so the same folder spelled differently still dedups.
        let third = alice
            .add_file("cat.png", "image/png", "/wiki/Cats/", data)
            .await
            .unwrap();
        assert_eq!(third, cid);
        assert_eq!(alice.files().len(), 1, "'/wiki/Cats/' is 'wiki/Cats'");
        assert_eq!(alice.download_file(&cid).await.unwrap(), data.to_vec());
    }

    #[tokio::test]
    async fn the_same_bytes_in_a_second_folder_are_relisted_not_restored() {
        // The same image attached to a second wiki page: a new listing, but it points at the
        // already-sealed blobs; no re-seal (which, being randomized, would store a second,
        // byte-different copy of the same content).
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let data = b"the bytes of a cat picture";
        let cid = alice
            .add_file("cat.png", "image/png", "wiki/Cats", data)
            .await
            .unwrap();
        let blobs = alice.sync().blob_cids();

        let same = alice
            .add_file("kitten.png", "image/png", "emoji", data)
            .await
            .unwrap();
        assert_eq!(same, cid, "same content, same address");

        let mut listed: Vec<(String, String)> = alice
            .files()
            .into_iter()
            .map(|e| (e.name, e.path))
            .collect();
        listed.sort();
        assert_eq!(
            listed,
            vec![
                ("cat.png".to_string(), "wiki/Cats".to_string()),
                ("kitten.png".to_string(), "emoji".to_string()),
            ],
            "two listings of one file"
        );
        assert_eq!(
            alice.sync().blob_cids().len(),
            blobs.len(),
            "the blob store did not grow"
        );
        assert_eq!(
            chunk_cids(&alice, &cid, "emoji"),
            chunk_cids(&alice, &cid, "wiki/Cats"),
            "the new listing names the SAME sealed chunk blobs"
        );
        assert!(alice.file_available(&cid), "and is available locally");
        assert_eq!(alice.download_file(&cid).await.unwrap(), data.to_vec());
    }

    #[tokio::test]
    async fn a_re_shared_chunked_file_stores_no_new_chunk_blobs() {
        // Dedup covers the multi-chunk path too: the manifest (and so every chunk blob) is reused
        // wholesale, which is what keeps a big re-shared file from doubling local storage.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let n = CHUNK_BYTES + 4242;
        let data: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
        let cid = alice
            .add_file("big.bin", "application/octet-stream", "", &data)
            .await
            .unwrap();
        let blobs = alice.sync().blob_cids().len();
        let chunks = chunk_cids(&alice, &cid, "");
        assert!(chunks.len() >= 2, "the file really is chunked");

        let same = alice
            .add_file("big-copy.bin", "application/octet-stream", "docs", &data)
            .await
            .unwrap();
        assert_eq!(same, cid);
        assert_eq!(alice.files().len(), 2, "listed twice");
        assert_eq!(
            alice.sync().blob_cids().len(),
            blobs,
            "no new chunk blob was stored"
        );
        assert_eq!(chunk_cids(&alice, &cid, "docs"), chunks);
        assert_eq!(
            alice.download_file(&cid).await.unwrap(),
            data,
            "and it still reassembles byte-for-byte"
        );
    }

    #[tokio::test]
    async fn unlisting_one_dedup_listing_leaves_the_other_downloadable() {
        // Two listings share one set of chunk blobs, so the delete-time GC must NOT reclaim them
        // while a sibling listing still references them.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let data = b"the bytes shared by two listings";
        let cid = alice
            .add_file("cat.png", "image/png", "wiki/Cats", data)
            .await
            .unwrap();
        alice
            .add_file("cat.png", "image/png", "emoji", data)
            .await
            .unwrap();
        let chunks = chunk_cids(&alice, &cid, "emoji");

        alice.delete_file_at(&cid, "emoji").await.unwrap();

        let files = alice.files();
        assert_eq!(files.len(), 1, "only the emoji listing went");
        assert_eq!(files[0].path, "wiki/Cats");
        assert!(
            chunks.iter().all(|c| alice.sync().has_blob(c)),
            "the shared chunk blobs survive the GC"
        );
        assert!(alice.file_available(&cid));
        assert_eq!(
            alice.download_file(&cid).await.unwrap(),
            data.to_vec(),
            "the surviving listing still downloads and verifies"
        );

        // Unlisting the last listing does reclaim them (nothing references them any more).
        alice.delete_file(&cid).await.unwrap();
        assert!(alice.files().is_empty());
        for c in &chunks {
            assert!(!alice.sync().has_blob(c), "now orphaned, so GC'd");
        }
        assert!(
            alice.delete_file_at(&cid, "wiki/Cats").await.is_err(),
            "unlisting what is not listed errors"
        );
    }

    #[tokio::test]
    async fn different_bytes_under_the_same_name_are_two_distinct_files() {
        // Dedup keys on content, never on the name: same name + folder, different bytes ⇒ two
        // entries with two addresses, each independently downloadable.
        let mut alice = founder();
        alice.open_files().await.unwrap();
        let one = alice
            .add_file("note.txt", "text/plain", "docs", b"version one")
            .await
            .unwrap();
        let blobs = alice.sync().blob_cids().len();
        let two = alice
            .add_file("note.txt", "text/plain", "docs", b"version two")
            .await
            .unwrap();

        assert_ne!(one, two, "different content, different address");
        assert_eq!(alice.files().len(), 2, "no false dedup on the name");
        assert!(
            alice.sync().blob_cids().len() > blobs,
            "the second file really was stored"
        );
        assert_eq!(
            alice.download_file(&one).await.unwrap(),
            b"version one".to_vec()
        );
        assert_eq!(
            alice.download_file(&two).await.unwrap(),
            b"version two".to_vec()
        );
    }

    // ---------- circulation expiry + wiki pinning ----------
    //
    // Every assertion below is about *recorded metadata*: nothing in the stack drops, evicts or
    // deletes an expired file yet (see `FileExpiry`). These tests pin the inputs a future
    // retention pass will consume.

    /// A founder whose clock the test drives, so expiry stamps are exact.
    fn founder_with_clock(clock: &ManualClock) -> Server<MemNetwork, ChaCha20Rng> {
        let hub = Hub::new();
        Server::found(
            hub.join(PeerId::from_u64(1)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(clock.clone()),
            "alice",
        )
        .unwrap()
    }

    #[tokio::test]
    async fn a_shared_file_is_stamped_with_a_one_month_circulation_expiry() {
        let clock = ManualClock::new(T0);
        let mut alice = founder_with_clock(&clock);
        alice.open_files().await.unwrap();
        alice
            .add_file("doc.txt", "text/plain", "docs", b"hello")
            .await
            .unwrap();

        let entry = alice.files().remove(0);
        assert!(
            entry.author_verified,
            "new listings bind their origin device"
        );
        assert_eq!(
            entry.expires,
            FileExpiry::At(T0 + FILE_EXPIRY_DEFAULT_MS),
            "stamped from the INJECTED clock, one month out"
        );
        assert_eq!(
            entry.expires.deadline_ms(),
            Some(T0 + FILE_EXPIRY_DEFAULT_MS)
        );
        assert!(entry.expires.is_recorded());
    }

    #[tokio::test]
    async fn a_legacy_file_entry_without_an_expiry_decodes_as_not_recorded() {
        // A listing written before the field existed: same doc shape, no `exp` key. It must
        // decode + round-trip intact, and read back as "not recorded"; NOT as keep-forever,
        // which is a promise nobody made.
        let clock = ManualClock::new(T0);
        let mut alice = founder_with_clock(&clock);
        alice.open_files().await.unwrap();
        let cid = alice
            .add_file("doc.txt", "text/plain", "docs", b"hello")
            .await
            .unwrap();
        let legacy_ref = alice.files()[0].file_ref.clone();
        let author = alice.my_fingerprint();

        // Append an entry the way a pre-expiry peer would have: no `exp` key at all.
        alice
            .sync
            .post(DocType::FileIndex, FILE_INDEX_DOC, |d| {
                write_file_entry(
                    d,
                    "legacy.txt",
                    &author,
                    "docs",
                    &legacy_ref,
                    FileExpiry::Unrecorded,
                    None,
                )
            })
            .await
            .unwrap();

        let files = alice.files();
        assert_eq!(files.len(), 2);
        let legacy = files.iter().find(|e| e.name == "legacy.txt").unwrap();
        assert_eq!(legacy.expires, FileExpiry::Unrecorded);
        assert!(!legacy.expires.is_recorded(), "absent != keep-forever");
        assert_eq!(legacy.expires.deadline_ms(), None);
        // The rest of the entry survived the new field being added to the schema, and the
        // content is still downloadable through the legacy listing.
        assert_eq!(legacy.path, "docs");
        assert_eq!(legacy.author, author);
        assert!(
            !legacy.author_verified,
            "legacy author labels are not cryptographic proof"
        );
        assert_eq!(legacy.cid, cid.as_bytes().to_vec());
        assert_eq!(alice.download_file(&cid).await.unwrap(), b"hello".to_vec());
        // The modern entry alongside it is unaffected.
        let modern = files.iter().find(|e| e.name == "doc.txt").unwrap();
        assert_eq!(modern.expires, FileExpiry::At(T0 + FILE_EXPIRY_DEFAULT_MS));
        assert!(modern.author_verified);
    }

    #[tokio::test]
    async fn a_file_origin_attestation_rejects_relabelled_metadata() {
        let clock = ManualClock::new(T0);
        let mut alice = founder_with_clock(&clock);
        alice.open_files().await.unwrap();
        alice
            .add_file("safe.png", "image/png", "embed", b"image bytes")
            .await
            .unwrap();
        let original = alice.files().remove(0);
        let (public_key, signature) = alice
            .file_entry_attestation(
                &original.name,
                &original.author,
                &original.path,
                &original.file_ref,
            )
            .unwrap();
        let group_id = alice.sync.group_id();
        let canonical = file_entry_attestation_payload(
            &group_id,
            &original.name,
            &original.author,
            &original.path,
            &original.file_ref,
        );
        assert!(verify_with_public_bytes(
            &public_key,
            &canonical,
            &signature
        ));
        let mut changed_ref = original.file_ref.clone();
        changed_ref.push(0);
        for (field, payload) in [
            (
                "group",
                file_entry_attestation_payload(
                    b"another group",
                    &original.name,
                    &original.author,
                    &original.path,
                    &original.file_ref,
                ),
            ),
            (
                "name",
                file_entry_attestation_payload(
                    &group_id,
                    "renamed.png",
                    &original.author,
                    &original.path,
                    &original.file_ref,
                ),
            ),
            (
                "author",
                file_entry_attestation_payload(
                    &group_id,
                    &original.name,
                    "ffffffff",
                    &original.path,
                    &original.file_ref,
                ),
            ),
            (
                "path",
                file_entry_attestation_payload(
                    &group_id,
                    &original.name,
                    &original.author,
                    "another-folder",
                    &original.file_ref,
                ),
            ),
            (
                "file reference",
                file_entry_attestation_payload(
                    &group_id,
                    &original.name,
                    &original.author,
                    &original.path,
                    &changed_ref,
                ),
            ),
        ] {
            assert!(
                !verify_with_public_bytes(&public_key, &payload, &signature),
                "changing {field} must invalidate the listing attestation"
            );
        }
        let mut changed_key = public_key.clone();
        changed_key[0] ^= 1;
        assert!(!verify_with_public_bytes(
            &changed_key,
            &canonical,
            &signature
        ));
        let mut changed_signature = signature;
        changed_signature[0] ^= 1;
        assert!(!verify_with_public_bytes(
            &public_key,
            &canonical,
            &changed_signature
        ));
        let author = original.author.clone();
        let file_ref = original.file_ref.clone();
        alice
            .sync
            .post(DocType::FileIndex, FILE_INDEX_DOC, |d| {
                // A modified client copied a valid attestation but changed the displayed name.
                write_file_entry(
                    d,
                    "malicious.png",
                    &author,
                    "embed",
                    &file_ref,
                    FileExpiry::Never,
                    Some((&public_key, &signature)),
                )
            })
            .await
            .unwrap();

        let tampered = alice
            .files()
            .into_iter()
            .find(|entry| entry.name == "malicious.png")
            .unwrap();
        assert!(!tampered.author_verified);
    }

    #[tokio::test]
    async fn file_index_materialization_and_publication_capacity_are_deterministically_bounded() {
        let clock = ManualClock::new(T0);
        let mut alice = founder_with_clock(&clock);
        alice.open_files().await.unwrap();
        alice
            .add_file("safe.png", "image/png", "embed", b"image bytes")
            .await
            .unwrap();
        let original = alice.files().remove(0);
        let (public_key, signature) = alice
            .file_entry_attestation(
                &original.name,
                &original.author,
                &original.path,
                &original.file_ref,
            )
            .unwrap();
        let group_id = alice.sync.group_id();

        let mut hostile = AutoCommit::new();
        for _ in 0..(MAX_FILE_ENTRIES + 7) {
            write_file_entry(
                &mut hostile,
                &original.name,
                &original.author,
                &original.path,
                &original.file_ref,
                FileExpiry::Never,
                Some((&public_key, &signature)),
            )
            .unwrap();
        }
        assert_eq!(raw_file_index_row_count(&hostile), MAX_FILE_ENTRIES + 7);
        let materialized = read_file_entries(&hostile, &group_id);
        assert_eq!(materialized.len(), MAX_FILE_ENTRIES);
        assert!(materialized.iter().all(|entry| entry.author_verified));

        let mut malformed = AutoCommit::new();
        let list = malformed.put_object(ROOT, FILES, ObjType::List).unwrap();
        for i in 0..MAX_FILE_ENTRIES {
            malformed.insert_object(&list, i, ObjType::Map).unwrap();
        }
        assert_eq!(raw_file_index_row_count(&malformed), MAX_FILE_ENTRIES);
        assert!(read_file_entries(&malformed, &group_id).is_empty());
    }

    #[test]
    fn file_index_mutations_never_scan_or_change_the_unbounded_tail() {
        let bytes = b"bounded mutation";
        let (chunk, _) = catcoms_storage::seal_file(
            bytes,
            "text/plain",
            &[9u8; 32],
            &mut ChaCha20Rng::seed_from_u64(88),
        )
        .unwrap();
        let manifest = FileManifest {
            plaintext_cid: Cid::of(bytes),
            total_size: bytes.len() as u64,
            mime: "text/plain".into(),
            chunks: vec![chunk],
        };
        manifest.validate_layout().unwrap();
        let file_ref = manifest.encode();
        let cid = manifest.plaintext_cid;
        let mut doc = AutoCommit::new();
        for _ in 0..(MAX_FILE_ENTRIES + 7) {
            write_file_entry(
                &mut doc,
                "bounded.txt",
                "author",
                "docs",
                &file_ref,
                FileExpiry::Never,
                None,
            )
            .unwrap();
        }

        set_file_entry_expiry(&mut doc, cid.as_bytes(), "docs", FileExpiry::At(T0 + 1)).unwrap();
        let (_, list) = doc.get(ROOT, FILES).unwrap().unwrap();
        for index in 0..MAX_FILE_ENTRIES {
            let (_, entry) = doc.get(&list, index).unwrap().unwrap();
            assert_eq!(expiry_field(&doc, &entry), FileExpiry::At(T0 + 1));
        }
        for index in MAX_FILE_ENTRIES..(MAX_FILE_ENTRIES + 7) {
            let (_, entry) = doc.get(&list, index).unwrap().unwrap();
            assert_eq!(expiry_field(&doc, &entry), FileExpiry::Never);
        }

        delete_file_entry(&mut doc, cid.as_bytes(), Some("docs")).unwrap();
        assert_eq!(raw_file_index_row_count(&doc), 7);
        let (_, list) = doc.get(ROOT, FILES).unwrap().unwrap();
        for index in 0..7 {
            let (_, entry) = doc.get(&list, index).unwrap().unwrap();
            assert_eq!(expiry_field(&doc, &entry), FileExpiry::Never);
        }
    }

    #[tokio::test]
    async fn oversized_file_listing_fields_fail_closed() {
        let clock = ManualClock::new(T0);
        let mut alice = founder_with_clock(&clock);
        alice.open_files().await.unwrap();
        let oversized_name = "n".repeat(MAX_FILE_NAME_BYTES + 1);
        assert!(alice
            .add_file(&oversized_name, "image/png", "embed", b"bytes")
            .await
            .is_err());
        assert_eq!(
            alice.clear_staged_uploads(),
            0,
            "bounds are checked before sealing"
        );
    }

    #[tokio::test]
    async fn keep_forever_round_trips_and_a_non_uploader_member_is_refused() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let clock = ManualClock::new(T0);
        let mut alice = Server::found(
            hub.join(alice_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(clock.clone()),
            "alice",
        )
        .unwrap();
        alice.subscribe_control().await.unwrap();

        let invite = alice.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (bob, _) = tokio::join!(
            Server::join(
                hub.join(PeerId::from_u64(2)),
                MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(2),
                Box::new(clock.clone()),
                "bob",
                alice_peer,
                &invite,
            ),
            alice.sync_once(),
        );
        let mut bob = bob.unwrap();

        alice.open_files().await.unwrap();
        let cid = alice
            .add_file("doc.txt", "text/plain", "docs", b"hello")
            .await
            .unwrap();

        // Bob really can see the listing; so the refusal below is the ROLE gate, not "no such
        // file" standing in for it.
        bob.open_files().await.unwrap();
        let _ = tokio::join!(bob.request_files_catchup(alice_peer), alice.sync_once());
        assert_eq!(bob.files().len(), 1, "bob sees alice's listing");
        assert!(!bob.is_owner());

        let err = bob.set_file_expiry(&cid, "docs", None).await;
        assert!(
            matches!(err, Err(AppError::Invalid(ref m)) if m.contains("uploader")),
            "a plain member who is not the uploader cannot change expiry: {err:?}"
        );
        assert_eq!(
            alice.files()[0].expires,
            FileExpiry::At(T0 + FILE_EXPIRY_DEFAULT_MS),
            "the refused call changed nothing"
        );

        // The uploader (also the owner here) keeps it forever, then sets an explicit date back.
        alice.set_file_expiry(&cid, "docs", None).await.unwrap();
        assert_eq!(alice.files()[0].expires, FileExpiry::Never);
        assert_eq!(alice.files()[0].expires.deadline_ms(), None);
        assert!(
            alice.files()[0].expires.is_recorded(),
            "keep-forever is a RECORDED decision, unlike a legacy entry"
        );

        alice
            .set_file_expiry(&cid, "docs", Some(T0 + 5_000))
            .await
            .unwrap();
        assert_eq!(alice.files()[0].expires, FileExpiry::At(T0 + 5_000));

        // A listing that isn't there (wrong folder) errors rather than silently succeeding.
        assert!(matches!(
            alice.set_file_expiry(&cid, "elsewhere", None).await,
            Err(AppError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn expiry_is_per_listing_and_a_dedup_relisting_gets_a_fresh_one() {
        // Content dedup lists the same bytes twice against one set of sealed blobs. Each listing
        // is its own act of sharing, so each carries its own deadline; the second must be
        // stamped at the time IT was shared, not inherited from the twin.
        let clock = ManualClock::new(T0);
        let mut alice = founder_with_clock(&clock);
        alice.open_files().await.unwrap();
        let data = b"shared-bytes";
        let cid = alice
            .add_file("cat.png", "image/png", "wiki/Cats", data)
            .await
            .unwrap();
        let blobs = alice.sync().blob_cids().len();

        clock.advance_ms(60_000);
        let again = alice
            .add_file("cat.png", "image/png", "emoji", data)
            .await
            .unwrap();
        assert_eq!(cid, again, "same content, same address");
        assert_eq!(
            alice.sync().blob_cids().len(),
            blobs,
            "the dedup path stored no new blobs"
        );

        let files = alice.files();
        assert_eq!(files.len(), 2);
        let wiki = files.iter().find(|e| e.path == "wiki/Cats").unwrap();
        let emoji = files.iter().find(|e| e.path == "emoji").unwrap();
        assert_eq!(wiki.expires, FileExpiry::At(T0 + FILE_EXPIRY_DEFAULT_MS));
        assert_eq!(
            emoji.expires,
            FileExpiry::At(T0 + 60_000 + FILE_EXPIRY_DEFAULT_MS),
            "the re-listing is stamped fresh, not inherited"
        );

        // Adjusting one listing leaves the other alone.
        alice
            .set_file_expiry(&cid, "wiki/Cats", None)
            .await
            .unwrap();
        let files = alice.files();
        assert_eq!(
            files
                .iter()
                .find(|e| e.path == "wiki/Cats")
                .unwrap()
                .expires,
            FileExpiry::Never
        );
        assert_eq!(
            files.iter().find(|e| e.path == "emoji").unwrap().expires,
            FileExpiry::At(T0 + 60_000 + FILE_EXPIRY_DEFAULT_MS),
            "the other folder's listing keeps its own deadline"
        );
    }

    #[tokio::test]
    async fn wiki_pinned_cids_catches_both_markers_and_unpins_when_the_page_changes() {
        let clock = ManualClock::new(T0);
        let mut alice = founder_with_clock(&clock);
        alice.open_files().await.unwrap();
        alice.open_wiki().await.unwrap();
        let embedded = alice
            .add_file("cat.png", "image/png", "wiki/Cats", b"CAT")
            .await
            .unwrap();
        let chipped = alice
            .add_file("notes.pdf", "application/pdf", "docs", b"NOTES")
            .await
            .unwrap();
        let lonely = alice
            .add_file("nobody.bin", "application/octet-stream", "", b"NOBODY")
            .await
            .unwrap();

        assert!(alice.wiki_pinned_cids().is_empty(), "no pages yet");

        // Both grammars pin: `![alt](cid:HEX)` embeds and `[label](file:HEX)` ref chips.
        alice
            .write_wiki_page(
                "Cats",
                &format!(
                    "Look: ![a cat](cid:{})\n\nSee also [the notes](file:{}).",
                    embedded.to_hex(),
                    chipped.to_hex()
                ),
            )
            .await
            .unwrap();
        let pinned = alice.wiki_pinned_cids();
        assert!(pinned.contains(&embedded.to_hex()), "embed marker pins");
        assert!(pinned.contains(&chipped.to_hex()), "ref chip pins");
        assert!(!pinned.contains(&lonely.to_hex()), "unreferenced, unpinned");
        assert_eq!(pinned.len(), 2);

        // Uppercase hex in the body still matches the lowercase cid form.
        alice
            .write_wiki_page(
                "Shouty",
                &format!("![A CAT](cid:{})", embedded.to_hex().to_uppercase()),
            )
            .await
            .unwrap();
        assert!(alice.wiki_pinned_cids().contains(&embedded.to_hex()));
        alice.delete_wiki_page("Shouty").await.unwrap();

        // Editing the embed out un-pins with no bookkeeping: the set is derived, never stored.
        alice
            .write_wiki_page("Cats", "Now the page has no media at all.")
            .await
            .unwrap();
        assert!(
            alice.wiki_pinned_cids().is_empty(),
            "dropping the markers un-pins both files"
        );

        // ...and so does deleting the page outright.
        alice
            .write_wiki_page("Cats", &format!("![a cat](cid:{})", embedded.to_hex()))
            .await
            .unwrap();
        assert_eq!(alice.wiki_pinned_cids().len(), 1);
        alice.delete_wiki_page("Cats").await.unwrap();
        assert!(
            alice.wiki_pinned_cids().is_empty(),
            "a deleted page pins nothing"
        );
    }

    #[tokio::test]
    async fn file_usage_counts_wiki_status_and_chat_references() {
        let clock = ManualClock::new(T0);
        let mut alice = founder_with_clock(&clock);
        alice.open_files().await.unwrap();
        alice.open_wiki().await.unwrap();
        alice.open_status().await.unwrap();
        alice.open_channel(GENERAL).await.unwrap();
        let cid = alice
            .add_file("cat.png", "image/png", "wiki/Cats", b"CAT")
            .await
            .unwrap();
        let other = alice
            .add_file("dog.png", "image/png", "wiki/Dogs", b"DOG")
            .await
            .unwrap();
        let hex = cid.to_hex();

        assert_eq!(alice.file_usage(&cid), FileUsage::default());
        assert!(!alice.file_usage(&cid).wiki_pinned());
        assert!(alice.file_usage(&cid).is_empty());

        alice
            .write_wiki_page("Cats", &format!("![a cat](cid:{hex})"))
            .await
            .unwrap();
        alice
            .write_wiki_page("Index", &format!("[the cat](file:{hex})"))
            .await
            .unwrap();
        alice
            .write_wiki_page("Dogs", &format!("![a dog](cid:{})", other.to_hex()))
            .await
            .unwrap();
        alice
            .post_status(&format!("new pic ![a cat](cid:{hex})"))
            .await
            .unwrap();
        alice.post_status("nothing to see here").await.unwrap();
        alice
            .send_message(GENERAL, &format!("look ![a cat](cid:{hex})"))
            .await
            .unwrap();
        alice
            .send_message(GENERAL, &format!("and again [cat](file:{hex})"))
            .await
            .unwrap();
        alice
            .send_message(GENERAL, "unrelated chatter")
            .await
            .unwrap();

        let usage = alice.file_usage(&cid);
        assert_eq!(
            usage.wiki_pages,
            vec!["Cats".to_string(), "Index".to_string()],
            "both grammars count, sorted, and the Dogs page is not listed"
        );
        assert_eq!(usage.status_count, 1);
        assert_eq!(usage.chat_count, 2);
        assert!(usage.wiki_pinned());
        assert!(!usage.is_empty());

        // The other file's usage is independent.
        let dog = alice.file_usage(&other);
        assert_eq!(dog.wiki_pages, vec!["Dogs".to_string()]);
        assert_eq!((dog.status_count, dog.chat_count), (0, 0));
    }

    #[test]
    fn the_marker_scanner_reads_both_grammars_and_ignores_near_misses() {
        let mut out = HashSet::new();
        let a = "ab".repeat(32); // 64 hex chars
        scan_file_markers(
            &format!(
                "![x](cid:{a}) [y](file:{a}) [z](file:) ![w](cid:zzzz) [v](file:{a} \
                 [u](status:{a}) ![t](cid:{})",
                a.to_uppercase()
            ),
            &mut out,
        );
        assert_eq!(
            out,
            HashSet::from([a.clone()]),
            "both grammars hit (case-folded); an empty/non-hex/unterminated marker and a \
             status ref do not"
        );
    }
}
