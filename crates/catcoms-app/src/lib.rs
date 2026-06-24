//! The **product model** — a UI-facing facade over the CatComs stack.
//!
//! Everything below `catcoms-sync` speaks in protocol terms (MLS groups, sealed ops,
//! automerge docs, transport peers). A UI does not want any of that; it wants
//! *servers*, *channels*, *messages*, and *members*. This crate is that translation
//! layer: a thin, typed boundary the desktop/mobile UI (and the Tauri command bridge)
//! is built against, so the GUI never touches MLS or automerge directly.
//!
//! It owns the **canonical channel-message schema** (a channel document is a list of
//! `{author, text}` maps) — previously poked inline in `catcomsctl`, now defined once
//! here so the protocol and the UI agree on what a chat message is.
//!
//! Scope (slice 8a): a single [`Server`] (one [`ChannelSync`]), driven explicitly via
//! [`Server::sync_once`]. The background run-loop + live event stream and multi-server
//! management land with the Tauri bridge (8b), where the real async runtime lives.

use std::collections::HashMap;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, AutomergeError, ObjId, ObjType, ReadDoc, ScalarValue, Value, ROOT};
use catcoms_crypto::DeviceId;
use catcoms_mls::{InviteToken, MlsDevice, MlsError, ServerGroup};
use catcoms_rt::{Clock, CryptoRngCore, DiscoveredPeer, MeshTransport, PeerId};
use catcoms_storage::{BlobStore, Cid, FileManifest};
pub use catcoms_sync::peer_addrs_from_snapshot;
use catcoms_sync::{
    fingerprint, read_published_roster, request_join, ChannelSync, SyncError, ROLES_DOC,
};
use catcoms_wire::DocType;
use thiserror::Error;

mod actor;
pub mod store;
pub use actor::{spawn, AppCommand, AppEvent, ServerActor};
pub use store::{ServerRecord, ServerStore};

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
    /// A product-layer validation error (e.g. an over-large avatar).
    #[error("{0}")]
    Invalid(String),
    /// The join did not finalize before the wall-clock deadline — e.g. an Option-C admin invite
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
    /// The author's device fingerprint (resolve to a name/style via [`Server::profiles`]).
    pub author: String,
    /// The message text.
    pub text: String,
    /// Send time in epoch-millis (the sender's injected clock; `0` if absent).
    pub ts: u64,
}

/// Deterministically derive a channel's document id from its **name**, so any two
/// members who open the same channel name converge on the same channel — IRC-style name
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

/// Wall-clock ceiling on a join handshake. Past this the joiner gives up (and the user can
/// retry) — so an Option-C admin invite whose owner never comes online can't wedge it forever.
const JOIN_TIMEOUT_SECS: u64 = 120;

// --- the canonical channel-document schema ----------------------------------
// A channel doc is `{ messages: [ { author: str, text: str } ] }`.

const MESSAGES: &str = "messages";
const AUTHOR: &str = "author";
const TEXT: &str = "text";
const TS: &str = "ts";

/// Append a `{author, text, ts}` message to a channel document (the canonical edit).
pub fn append_message(
    doc: &mut AutoCommit,
    author: &str,
    text: &str,
    ts: u64,
) -> Result<(), AutomergeError> {
    let list = match doc.get(ROOT, MESSAGES)? {
        Some((Value::Object(ObjType::List), id)) => id,
        _ => doc.put_object(ROOT, MESSAGES, ObjType::List)?,
    };
    let index = doc.length(&list);
    let msg = doc.insert_object(&list, index, ObjType::Map)?;
    doc.put(&msg, AUTHOR, author)?;
    doc.put(&msg, TEXT, text)?;
    doc.put(&msg, TS, ts as i64)?;
    Ok(())
}

/// Materialize a channel document into the UI's ordered message list.
pub fn read_messages(doc: &AutoCommit) -> Vec<ChatMessage> {
    let mut out = Vec::new();
    if let Ok(Some((Value::Object(ObjType::List), list))) = doc.get(ROOT, MESSAGES) {
        for i in 0..doc.length(&list) {
            if let Ok(Some((Value::Object(ObjType::Map), msg))) = doc.get(&list, i) {
                out.push(ChatMessage {
                    author: str_field(doc, &msg, AUTHOR),
                    text: str_field(doc, &msg, TEXT),
                    ts: int_field(doc, &msg, TS),
                });
            }
        }
    }
    out
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

// --- member profiles --------------------------------------------------------
//
// One shared CRDT document per server (`DocType::Profile`, id `PROFILE_DOC`), a map
// keyed by member **device fingerprint** → `{ name, color, font, effect }`. Each member
// writes only their *own* fingerprint's entry by convention (the op is inner-signed by
// the author's device; enforcing "an op may only touch the author's own key" is a later
// hardening — a malicious member overwriting another's profile is low-stakes and
// detectable, not a confidentiality/integrity break).

/// The reserved document id for the per-server profile document.
const PROFILE_DOC: u128 = 0;
const P_NAME: &str = "name";
const P_COLOR: &str = "color";
const P_FONT: &str = "font";
const P_EFFECT: &str = "effect";
const P_AVATAR_CID: &str = "avatar_cid";

/// Maximum avatar image size accepted by [`Server::set_profile`]. Avatars are stored by
/// **content address** in the blob store (not inline in the gossiped profile document)
/// and fetched on demand over the mesh; this caps the blob the UI's downscaled ~128px
/// JPEG produces.
pub const MAX_AVATAR_BYTES: usize = 64 * 1024;

/// Max avatar blobs fetched per [`Server::fetch_missing_avatars`] pass — bounds how long
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
    /// The avatar image bytes, resolved from its content address against the local blob
    /// store (empty if unset or not yet fetched). The UI produces a downscaled JPEG.
    pub avatar: Vec<u8>,
}

/// An internal profile record straight from the document: the avatar is its **content
/// address** (CID bytes), resolved to image bytes against the blob store by [`Server`].
struct ProfileRecord {
    name: String,
    color: String,
    font: String,
    effect: String,
    avatar_cid: Vec<u8>,
}

/// Write a member's own profile entry. The avatar is referenced by **content address**
/// (`avatar_cid`), not stored inline — so the gossiped profile document stays tiny and the
/// image is fetched on demand over the mesh.
fn write_profile(
    doc: &mut AutoCommit,
    fp: &str,
    p: &Profile,
    avatar_cid: &[u8],
) -> Result<(), AutomergeError> {
    let entry = match doc.get(ROOT, fp)? {
        Some((Value::Object(ObjType::Map), id)) => id,
        _ => doc.put_object(ROOT, fp, ObjType::Map)?,
    };
    doc.put(&entry, P_NAME, p.name.as_str())?;
    doc.put(&entry, P_COLOR, p.color.as_str())?;
    doc.put(&entry, P_FONT, p.font.as_str())?;
    doc.put(&entry, P_EFFECT, p.effect.as_str())?;
    doc.put(
        &entry,
        P_AVATAR_CID,
        ScalarValue::Bytes(avatar_cid.to_vec()),
    )?;
    Ok(())
}

/// Materialize the profile document into `fingerprint -> ProfileRecord` (avatars still as
/// content addresses; [`Server::profiles`] resolves them against the blob store).
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
                    avatar_cid: bytes_field(doc, &entry, P_AVATAR_CID),
                },
            );
        }
    }
    out
}

/// Parse a stored avatar content address (32 bytes) into a [`Cid`] (`None` if absent/bad).
fn parse_avatar_cid(bytes: &[u8]) -> Option<Cid> {
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some(Cid::from_bytes(arr))
}

// --- fileshare: a per-server file index --------------------------------------
//
// One shared CRDT document per server (`DocType::FileIndex`, id `FILE_INDEX_DOC`): an
// append-only list of file entries `{ name, size, mime, cid, author }`. The bytes live in
// the blob store and are fetched on demand over the mesh (8l); only the small metadata
// gossips in the (encrypted) index. NOTE: blobs are stored plaintext at rest and served
// members-only — per-file encryption-at-rest (`catcoms-storage::seal_file`) is a hardening
// follow-up; the index metadata is already confidential (it is an encrypted CRDT doc).

/// The reserved document id for the per-server file index.
const FILE_INDEX_DOC: u128 = 0;
/// The reserved document id for the per-server status feed (`DocType::Status`).
const STATUS_DOC: u128 = 0;
/// The reserved document id for the per-server wiki (`DocType::Wiki`) — one doc that is a
/// map of page name → page body.
const WIKI_DOC: u128 = 0;

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
fn read_wiki_map(doc: &AutoCommit) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for key in doc.keys(ROOT) {
        if let Ok(Some((Value::Object(ObjType::Text), id))) = doc.get(ROOT, &key) {
            if let Ok(body) = doc.text(&id) {
                out.insert(key, body);
            }
        }
    }
    out
}
// --- member roles (Phase 10h + item 3) -------------------------------------
//
// The **owner** is the MLS designated committer (the founder — cryptographically anchored, not
// stored); everyone else is a plain member or an **admin**.
//
// ENFORCEMENT (item 3 — replay-proof revocation; see `docs/design-grant-revocation.md`):
//
// * **Owner** is the MLS designated committer (lowest leaf index) — only that device can act
//   as committer, so the owner is cryptographically anchored, not a stored/forgeable field.
//   It is NOT sticky: it follows the lowest *live* leaf, so if the founder ever leaves the
//   group, ownership (and admin-granting power) passes to the next-lowest member. (Founder
//   removal is not wired into the desktop app yet, so this is latent.) A new owner starts with
//   an empty roster (prior grants lapse until re-granted) — no stale-grant inheritance.
//
// * **Admin** authority is the **owner's local authoritative roster** (`ChannelSync::admin_roster`,
//   persisted in the snapshot). Because only the owner runs admission (Option C), the gate
//   (`inviter_is_authorized`) reads that local set, which a malicious member cannot write — so a
//   demoted admin re-adding/replaying its old grant into the shared CRDT can no longer
//   re-authorize itself (closes the old revocation-by-deletion residual). The owner publishes a
//   single owner-signed `roster` value into `DocType::MemberRoles` for **display only**; readers
//   verify the owner's signature, so a tampered copy is at worst cosmetic (never an admission).
//
// Residual: a tampered/stale published roster can transiently mislead *other members'* role
// badges (cosmetic, R4-class). The guarantee rests on single-committer admission — do NOT enable
// `max_committer_rank ≥ 1` (a second committer would re-introduce the replay surface).

// `ROLES_DOC`, `roster_payload`, `encode_roster`, `read_published_roster`, and `fingerprint` live
// in `catcoms-sync` (next to the admission gate that enforces them) and are re-exported here.

/// A member's effective role in a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The server owner (the MLS designated committer / founder).
    Owner,
    /// An admin (granted by the owner) — may mint invites.
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

/// Maximum file size accepted by [`Server::add_file`]. Chunked transfer splits a file into
/// [`CHUNK_BYTES`] pieces, so this is a whole-**file** cap (256 MiB), no longer the per-blob
/// transport limit. (GB-scale + background/streaming download is a follow-up.)
pub const MAX_FILE_BYTES: usize = 256 * 1024 * 1024;

/// Plaintext chunk size for large-file transfer. Chosen well under the blob-fetch response cap
/// (`MAX_BLOB_RESPONSE` = 16 MiB) so each *sealed* chunk (≈ chunk + ~40 B) fits one response.
const CHUNK_BYTES: usize = 8 * 1024 * 1024;

/// One shared file as the UI sees it. `cid` is the **whole-file plaintext** content address (raw
/// bytes) — the file's stable identity / download+embed handle (a chunked file has no single
/// ciphertext blob); `author` is the uploader's device fingerprint. The file's bytes are
/// end-to-end encrypted under the group file-wrap key (Phase 9h), chunk by chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// The file's display name.
    pub name: String,
    /// Plaintext size in bytes.
    pub size: u64,
    /// MIME type (best-effort; may be empty).
    pub mime: String,
    /// Content address of the **ciphertext** blob (raw 32 bytes) — the download handle.
    pub cid: Vec<u8>,
    /// The uploader's device fingerprint.
    pub author: String,
    /// A virtual folder path for organisation (`""` = root). Embeds live under
    /// `embed/<fp>`, wiki media under `wiki/<page>`, custom emoji under `emoji` (10c–10f).
    pub path: String,
    /// The encoded [`FileRef`] (wrapped per-file key + addresses) needed to decrypt. Carried
    /// in the encrypted index; not forwarded to the UI.
    pub file_ref: Vec<u8>,
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

/// Append a file entry (name + author + folder path + encoded `FileRef`) to the index doc.
fn write_file_entry(
    doc: &mut AutoCommit,
    name: &str,
    author: &str,
    path: &str,
    file_ref: &[u8],
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
    Ok(())
}

/// Materialize the file index document into the UI's file list (size/mime/cid come from the
/// decoded `FileRef`; entries with a malformed ref are skipped).
fn read_file_entries(doc: &AutoCommit) -> Vec<FileEntry> {
    let mut out = Vec::new();
    if let Ok(Some((Value::Object(ObjType::List), list))) = doc.get(ROOT, FILES) {
        for i in 0..doc.length(&list) {
            if let Ok(Some((Value::Object(ObjType::Map), entry))) = doc.get(&list, i) {
                let ref_bytes = bytes_field(doc, &entry, F_REF);
                if let Ok(manifest) = FileManifest::decode_or_legacy(&ref_bytes) {
                    out.push(FileEntry {
                        name: str_field(doc, &entry, F_NAME),
                        author: str_field(doc, &entry, F_AUTHOR),
                        size: manifest.total_size,
                        mime: manifest.mime.clone(),
                        // The file's identity is the whole-file PLAINTEXT cid (a chunked file has
                        // no single ciphertext blob); this is the download/embed handle.
                        cid: manifest.plaintext_cid.as_bytes().to_vec(),
                        path: str_field(doc, &entry, F_PATH),
                        file_ref: ref_bytes,
                    });
                }
            }
        }
    }
    out
}

/// Remove *every* index entry whose ciphertext CID matches `cid` (a no-op if none do). The
/// index is an append-only list with no dedup, so a concurrent double-add can leave more than
/// one entry for the same content; unlisting must remove them all. Iterating top-down keeps the
/// indices we have yet to visit stable as we delete. The content-addressed blob is left in
/// place; this only unlists the file.
fn delete_file_entry(doc: &mut AutoCommit, cid: &[u8]) -> Result<(), AutomergeError> {
    let list = match doc.get(ROOT, FILES)? {
        Some((Value::Object(ObjType::List), id)) => id,
        _ => return Ok(()),
    };
    for i in (0..doc.length(&list)).rev() {
        if let Some((Value::Object(ObjType::Map), entry)) = doc.get(&list, i)? {
            let ref_bytes = bytes_field(doc, &entry, F_REF);
            if let Ok(manifest) = FileManifest::decode_or_legacy(&ref_bytes) {
                if manifest.plaintext_cid.as_bytes() == cid {
                    doc.delete(&list, i)?;
                }
            }
        }
    }
    Ok(())
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
}

/// A UI-facing view of one member: a short device-id **fingerprint** (display names are
/// not shared on the wire yet, so the roster identifies members by their content-
/// addressed device id) and whether it is **this** device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberView {
    /// Short hex fingerprint of the member's device id (first 4 bytes).
    pub fingerprint: String,
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
        // Bound the whole join so a never-finalizing owner (an Option-C admin invite whose owner
        // stays offline) can't wedge the joiner forever; the sync layer stays runtime-agnostic.
        let (group, routing) = tokio::time::timeout(
            std::time::Duration::from_secs(JOIN_TIMEOUT_SECS),
            request_join(&transport, inviter, &device, invite),
        )
        .await
        .map_err(|_| AppError::JoinTimeout)??;
        Ok(Self {
            sync: ChannelSync::new_joined(transport, group, device, rng, clock, routing),
            display_name: display_name.into(),
            device_id,
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
        })
    }

    /// This server's MLS group id (stable across restarts) — keys its on-disk blob directory.
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

    /// Send a chat message to a channel. The message is **authored by this device's
    /// fingerprint**; the display name + styling are resolved from the author's profile
    /// at render time (so a profile change updates all of that member's messages).
    pub async fn send_message(&mut self, channel: u128, text: &str) -> Result<(), AppError> {
        let author = self.my_fingerprint();
        let ts = self.sync.now_ms();
        self.sync
            .post(DocType::Channel, channel, |d| {
                append_message(d, &author, text, ts)
            })
            .await?;
        Ok(())
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
    /// image (rejected if larger than [`MAX_AVATAR_BYTES`]) is stored in the blob store and
    /// referenced by content address — the gossiped document carries only the CID.
    pub async fn set_profile(&mut self, profile: Profile) -> Result<(), AppError> {
        if profile.avatar.len() > MAX_AVATAR_BYTES {
            return Err(AppError::Invalid(format!(
                "avatar too large: {} bytes (max {MAX_AVATAR_BYTES})",
                profile.avatar.len()
            )));
        }
        let avatar_cid = if profile.avatar.is_empty() {
            Vec::new()
        } else {
            self.sync.put_blob(&profile.avatar)?.as_bytes().to_vec()
        };
        let fp = self.my_fingerprint();
        self.sync
            .post(DocType::Profile, PROFILE_DOC, |d| {
                write_profile(d, &fp, &profile, &avatar_cid)
            })
            .await?;
        Ok(())
    }

    /// All known member profiles, keyed by device fingerprint. Each profile's avatar is
    /// resolved from its content address against the **local** blob store — members whose
    /// avatar blob has not been fetched yet (see [`Server::fetch_missing_avatars`]) come
    /// back with an empty `avatar`.
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
                (
                    fp,
                    Profile {
                        name: r.name,
                        color: r.color,
                        font: r.font,
                        effect: r.effect,
                        avatar,
                    },
                )
            })
            .collect()
    }

    /// Fetch any referenced avatar blobs we do not yet hold from the best known peer.
    /// Returns how many were newly fetched (so the caller can re-render). Call after the
    /// profile document changes (e.g. on join/convergence). Fetches at most
    /// [`MAX_AVATAR_FETCHES_PER_PASS`] missing avatars per call — since each fetch is a
    /// blocking mesh round-trip, this bounds how long a single pass can stall the actor, so
    /// a member churning many distinct avatar CIDs cannot freeze peers' event loops (the
    /// remainder are picked up on subsequent ticks).
    pub async fn fetch_missing_avatars(&mut self) -> Result<usize, AppError> {
        let cids: Vec<Cid> = self
            .sync
            .doc(DocType::Profile, PROFILE_DOC)
            .map(|d| read_profile_records(d.doc()))
            .unwrap_or_default()
            .values()
            .filter_map(|r| parse_avatar_cid(&r.avatar_cid))
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
    pub async fn add_file(
        &mut self,
        name: &str,
        mime: &str,
        path: &str,
        bytes: &[u8],
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
        // (9h — a fresh per-chunk content key) and store the ciphertext blob, then describe the
        // file by a manifest (the whole-file plaintext cid + the ordered chunk FileRefs). The
        // (encrypted) index carries the manifest; the blob fetch caps only per-chunk size. The
        // file's identity is its whole-file plaintext cid.
        let author = self.my_fingerprint();
        let folder = normalize_path(path);
        let plaintext_cid = Cid::of(bytes);
        let mut chunks = Vec::new();
        for chunk in bytes.chunks(CHUNK_BYTES) {
            let (file_ref, ciphertext) = self.sync.seal_file(chunk, mime)?;
            self.sync.put_blob(&ciphertext)?;
            chunks.push(file_ref);
        }
        if chunks.is_empty() {
            // An empty file is still one (empty) chunk, so the manifest always has >= 1.
            let (file_ref, ciphertext) = self.sync.seal_file(&[], mime)?;
            self.sync.put_blob(&ciphertext)?;
            chunks.push(file_ref);
        }
        let manifest = FileManifest {
            plaintext_cid,
            total_size: bytes.len() as u64,
            mime: mime.to_string(),
            chunks,
        };
        let ref_bytes = manifest.encode();
        self.sync
            .post(DocType::FileIndex, FILE_INDEX_DOC, |d| {
                write_file_entry(d, name, &author, &folder, &ref_bytes)
            })
            .await?;
        Ok(plaintext_cid)
    }

    /// The shared files listed in the index (metadata only; bytes are fetched on download).
    pub fn files(&self) -> Vec<FileEntry> {
        self.sync
            .doc(DocType::FileIndex, FILE_INDEX_DOC)
            .map(|d| read_file_entries(d.doc()))
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
    /// `(chunks_done, chunks_total)` — `(0, total)` first so the UI shows 0% immediately, then
    /// `(i+1, total)` after each chunk. The actor wires this to a `DownloadProgress` event so a
    /// large multi-chunk download shows a progress bar.
    pub async fn download_file_with_progress(
        &mut self,
        cid: &Cid,
        progress: Option<&tokio::sync::mpsc::Sender<(usize, usize)>>,
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
            let _ = p.send((0, total)).await;
        }
        let mut out = Vec::with_capacity(manifest.total_size as usize);
        for (i, chunk_ref) in manifest.chunks.iter().enumerate() {
            let ccid = chunk_ref.ciphertext_cid;
            // Only reach out to a peer if we don't already hold this chunk ourselves.
            if !self.sync.has_blob(&ccid) {
                self.sync.request_blob_best(&ccid).await?;
            }
            let Some(ciphertext) = self.sync.get_blob(&ccid) else {
                return Err(if self.sync.has_blob(&ccid) {
                    AppError::Invalid(format!(
                        "chunk {i} is stored on this device but could not be read (it may be corrupted)"
                    ))
                } else {
                    AppError::Invalid(format!(
                        "file not available yet — no connected peer has chunk {i}"
                    ))
                });
            };
            let chunk = self
                .sync
                .open_file(&ciphertext, chunk_ref)
                .map_err(|e| AppError::Invalid(format!("chunk {i} could not be decrypted: {e}")))?;
            out.extend_from_slice(&chunk);
            if let Some(p) = progress {
                let _ = p.send((i + 1, total)).await;
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

    /// Whether this device already holds **all** of the file's chunk blobs locally — i.e. it can
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
        match FileManifest::decode_or_legacy(&entry.file_ref) {
            Ok(manifest) => manifest
                .chunks
                .iter()
                .all(|c| self.sync.has_blob(&c.ciphertext_cid)),
            Err(_) => false,
        }
    }

    /// Remove a file from the shared index, and **garbage-collect its now-orphaned chunk blobs**
    /// from local storage. **Owner or admin only** — errors otherwise. The GC is **dedup-safe**:
    /// a chunk still referenced by another listed file (chunks are content-addressed, so two files
    /// can share one) is kept; only chunks no remaining manifest references are deleted (they're
    /// re-fetchable from any peer that still holds them). Like invites and member removal, the role
    /// gate is honest-client-enforced (the protocol residual is the same as those).
    pub async fn delete_file(&mut self, cid: &Cid) -> Result<(), AppError> {
        if !matches!(self.my_role(), Role::Owner | Role::Admin) {
            return Err(AppError::Invalid(
                "only an owner or admin can delete files".into(),
            ));
        }
        let raw = cid.as_bytes().to_vec();
        if !self.files().iter().any(|e| e.cid == raw) {
            return Err(AppError::Invalid("no such file".into()));
        }
        // Capture the chunk blobs of the file(s) being removed, BEFORE unlisting them.
        let removed_chunks: Vec<Cid> = self
            .files()
            .iter()
            .filter(|e| e.cid == raw)
            .filter_map(|e| FileManifest::decode_or_legacy(&e.file_ref).ok())
            .flat_map(|m| m.chunks.into_iter().map(|c| c.ciphertext_cid))
            .collect();
        self.sync
            .post(DocType::FileIndex, FILE_INDEX_DOC, |d| {
                delete_file_entry(d, &raw)
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

    /// Catch up the file index document from `peer` (e.g. right after joining).
    pub async fn request_files_catchup(&mut self, peer: PeerId) -> Result<usize, AppError> {
        Ok(self
            .sync
            .request_catchup(peer, DocType::FileIndex, FILE_INDEX_DOC)
            .await?)
    }

    /// Open (create/subscribe) the per-server **status feed** — a server-wide stream of
    /// short posts (announcements/activity), reusing the canonical message schema on its
    /// own document. Call once after founding/joining.
    pub async fn open_status(&mut self) -> Result<(), AppError> {
        self.sync.open_channel(DocType::Status, STATUS_DOC).await?;
        Ok(())
    }

    /// Post to the status feed (authored by this device's fingerprint, clock-stamped).
    pub async fn post_status(&mut self, text: &str) -> Result<(), AppError> {
        let author = self.my_fingerprint();
        let ts = self.sync.now_ms();
        self.sync
            .post(DocType::Status, STATUS_DOC, |d| {
                append_message(d, &author, text, ts)
            })
            .await?;
        Ok(())
    }

    /// The current status feed (oldest-first; the UI shows newest at the top).
    pub fn statuses(&self) -> Vec<ChatMessage> {
        self.sync
            .doc(DocType::Status, STATUS_DOC)
            .map(|d| read_messages(d.doc()))
            .unwrap_or_default()
    }

    /// Catch up the status feed from `peer` (e.g. right after joining).
    pub async fn request_status_catchup(&mut self, peer: PeerId) -> Result<usize, AppError> {
        Ok(self
            .sync
            .request_catchup(peer, DocType::Status, STATUS_DOC)
            .await?)
    }

    /// Open (create/subscribe) the per-server **wiki** document. Call once after
    /// founding/joining.
    pub async fn open_wiki(&mut self) -> Result<(), AppError> {
        self.sync.open_channel(DocType::Wiki, WIKI_DOC).await?;
        Ok(())
    }

    /// The full wiki as a `page name -> body` map.
    pub fn wiki_map(&self) -> HashMap<String, String> {
        self.sync
            .doc(DocType::Wiki, WIKI_DOC)
            .map(|d| read_wiki_map(d.doc()))
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

    /// Create or update a wiki page (last-writer-wins on the body).
    pub async fn write_wiki_page(&mut self, name: &str, body: &str) -> Result<(), AppError> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::Invalid("empty wiki page name".into()));
        }
        self.sync
            .post(DocType::Wiki, WIKI_DOC, |d| write_wiki_page(d, &name, body))
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

    // --- member roles / permissions (Phase 10h) ----------------------------

    /// Open (create/subscribe) the per-server member-roles document. Call once after
    /// founding/joining.
    pub async fn open_roles(&mut self) -> Result<(), AppError> {
        self.sync
            .open_channel(DocType::MemberRoles, ROLES_DOC)
            .await?;
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
    /// — the same source the admission gate uses, so a tampered/replayed CRDT copy can't mislead
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

    /// Grant or revoke admin for a member fingerprint. **Owner only** — errors otherwise. Updates
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
        self.sync.request_remove(&target).await?;
        Ok(())
    }

    /// The current materialized messages in a channel (empty if it is not open).
    pub fn messages(&self, channel: u128) -> Vec<ChatMessage> {
        self.sync
            .doc(DocType::Channel, channel)
            .map(|d| read_messages(d.doc()))
            .unwrap_or_default()
    }

    /// Mint a single-use invite to this server.
    pub fn mint_invite(
        &self,
        nonce: [u8; 16],
        expires_at_ms: u64,
        bootstrap: Vec<String>,
    ) -> Result<InviteToken, AppError> {
        self.require_invite_permission()?;
        Ok(self.sync.mint_invite(nonce, expires_at_ms, bootstrap)?)
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

    /// The roster — device ids of all current members.
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
        Ok(self.sync.run_once().await?)
    }

    // --- steady-state rendezvous discovery (driven by the actor) -----------------------------

    /// Configure the rendezvous nodes this member registers/discovers at, so the group re-finds
    /// itself after a restart (founder: the rendezvous it registered at; joiner: the invite's).
    pub fn set_rendezvous_nodes(&mut self, nodes: Vec<(String, Vec<u8>)>) {
        self.sync.set_rendezvous_nodes(nodes);
    }

    /// Whether steady-state rendezvous discovery is configured (so the actor drives its tick).
    pub fn has_rendezvous(&self) -> bool {
        self.sync.has_rendezvous()
    }

    /// Drive one steady-state discovery tick (re-register + re-discover at the rendezvous).
    pub async fn drive_discovery(&mut self) {
        self.sync.drive_discovery().await;
    }

    /// Await the next rendezvous-discovered peer (inert without rendezvous configured).
    pub async fn next_discovered(&mut self) -> Option<DiscoveredPeer> {
        self.sync.next_discovered().await
    }

    /// Dial a discovered peer if the [`DiscoveryPolicy`](catcoms_discovery::DiscoveryPolicy)
    /// approves it (never auto-dial; membership re-proven post-dial via PEX).
    pub async fn ingest_discovered(&mut self, d: DiscoveredPeer) {
        self.sync.ingest_discovered(d).await;
    }

    /// Advisory eclipse check — `true` if the node may be isolated (verify a member out of band).
    /// Never gates anything; the actor surfaces a changed verdict to the UI.
    pub fn observe_eclipse(&mut self) -> bool {
        self.sync.observe_eclipse()
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
    /// peer) — no need to name a peer. Lets either side pull the backlog of a channel the
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
    async fn a_server_survives_a_sealed_store_round_trip() {
        // The full 9f loop: found → post → snapshot → seal to disk → reopen → restore →
        // read the history back, all offline (a fresh transport, no peers).
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(7);

        let mut alice = founder();
        alice.open_channel(GENERAL).await.unwrap();
        alice.send_message(GENERAL, "persist me").await.unwrap();
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

        // The channel history, display name and roster all survived — read offline.
        let msgs = restored.messages(GENERAL);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "persist me");
        assert_eq!(restored.display_name(), "alice");
        assert_eq!(restored.member_count(), 1);
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
        assert_eq!(applied.unwrap(), 1, "Bob applied Alice's one message");

        let msgs = bob.messages(GENERAL);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "welcome!");
        // Authored by Alice's device fingerprint (the name resolves from her profile).
        assert_eq!(msgs[0].author, alice.my_fingerprint());
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
        } // store + alice dropped — simulate the app closing

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

        // The owner removes Bob — the MLS commit drops him from the roster.
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

        // A plain member cannot delete a file — the role gate rejects before anything else.
        assert!(matches!(
            bob.delete_file(&cid).await,
            Err(AppError::Invalid(_))
        ));
        assert_eq!(alice.files().len(), 1, "the file is still listed");

        // The owner deletes it — the listing is removed.
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
            avatar: vec![0xff, 0xd8, 0xff, 0x00, 1, 2, 3], // stand-in JPEG bytes
        };
        alice.set_profile(p.clone()).await.unwrap();

        let profiles = alice.profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles.get(&alice.my_fingerprint()), Some(&p));
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
        // The owner may write a grant, but a grant for a non-member never resolves to admin —
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
}
