//! Channel synchronization: replicate encrypted CRDT documents over a mesh.
//!
//! [`ChannelSync`] bridges the Phase-4 replication engine ([`EncryptedDoc`]) and
//! the Phase-0 transport seam ([`MeshTransport`]) — so it runs identically over
//! the in-memory test transport and the libp2p mesh. It:
//!
//! - subscribes each open document to a **blinded topic** derived from the group
//!   id (only members can compute it),
//! - publishes locally-authored sealed ops to that topic (live fan-out), and
//!   ingests inbound ones into the matching document,
//! - serves and requests **catch-up bundles** over request/response, so a member
//!   who was offline (or just opened the channel) converges from a current
//!   member's signed-op log re-sealed under the current epoch,
//! - fans out membership commits on a per-group **control topic** and, when a
//!   member misses one (unreliable delivery), recovers it via **commit
//!   catch-up** with ordered replay (Phase 6d-1b), and
//! - keeps a bounded, zeroized **past-epoch channel-key window** so an op that
//!   was in flight across an epoch boundary still decrypts instead of being
//!   silently dropped (Phase 6d-1b).
//!
//! Deferred to later mesh sub-blocks: rotating the blinded topic on member
//! removal (here it is stable per group+document), and concurrent-commit fork
//! resolution / the full proposal-commit linearization of membership changes
//! (here a single designated committer serializes membership changes).

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt;

use automerge::{AutoCommit, AutomergeError};
use bytes::Bytes;
use catcoms_crypto::{seal, unseal, verify_with_public_bytes, DeviceId, SealedBlob};
use catcoms_mls::{
    restore_server, serialize_key_package, snapshot_server, Incoming, InviteLedger, InviteToken,
    MlsDevice, ServerGroup,
};
use catcoms_replication::{EncryptedDoc, SealedOp};
use catcoms_rt::{Clock, CryptoRngCore, MeshTransport, PeerId, ProtocolId, Topic, TransportEvent};
use catcoms_storage::{
    open_file as open_file_fn, seal_file as seal_file_fn, BlobStore, Cid, FileRef, MemoryBlobStore,
    StorageError,
};
use catcoms_wire::{Decoder, DocType, Encoder};
use thiserror::Error;
use zeroize::Zeroizing;

mod roles;
// Re-export the role-authority logic so the product/UI layer (catcoms-app) reuses this exact,
// canonical implementation rather than keeping a second copy that could drift.
pub use roles::{
    encode_roster, fingerprint, read_published_roster, roster_payload, ROLES_DOC,
    ROLE_ROSTER_DOMAIN, ROSTER_KEY,
};

/// Request/response protocol (one RR protocol; the first payload byte selects the
/// request kind so it works over the single-protocol mesh node).
const RR_PROTOCOL: &str = "/catcoms/rr/1";
/// Request kind: catch-up (document history) request.
const KIND_CATCHUP: u8 = 0;
/// Request kind: join (admission) request.
const KIND_JOIN: u8 = 1;
/// Request kind: membership commit catch-up (missed-commit recovery, 6d-1b).
const KIND_COMMIT_CATCHUP: u8 = 2;
/// Request kind: committer→joiner Welcome delivery once a *staged* admission
/// resolves — the provisional-Welcome push for the two-phase join (6d-2a).
const KIND_WELCOME: u8 = 3;
/// Request kind: member **peer exchange** (PEX, 6e-3d-7) — a member asks another
/// member for the signed peer records it knows, so members supply each other with
/// dialable peers without any rendezvous (defeats single-rendezvous omission).
const KIND_PEX: u8 = 4;
/// Request kind: **blob fetch** by content address (8l) — a member asks another member
/// for a content-addressed blob (avatars, files), so large/shared binaries move off the
/// gossiped documents onto on-demand mesh fetch. Members-only and signed, like catch-up.
const KIND_BLOB_FETCH: u8 = 5;
/// Request kind: the owner delivering a finalized admission back to the **admin** that
/// requested it (admin invites, Option C): `invite_nonce ‖ welcome ‖ sealed_routing ‖
/// owner_sig`. The admin verifies the owner's signature, re-signs the join transcript, and
/// pushes the Welcome to the waiting joiner.
#[allow(dead_code)] // wired into the relay flow in the next slice
const KIND_ADMIT_RESULT: u8 = 6;
/// Ceiling on a blob **response** accepted from a serving peer before storing. The blob's
/// content address is re-verified on store (so a wrong blob is rejected regardless); this
/// only bounds memory. Mirrors the 16 MiB catch-up ceiling.
const MAX_BLOB_RESPONSE: usize = 16 * 1024 * 1024;
/// Minimum interval (ms, injected clock) between blob responses *served* to the same
/// requesting **member** — a rate limit, since a 32-byte CID request can elicit a
/// response up to `MAX_BLOB_RESPONSE` (the strongest amplifier in the system) plus a
/// per-request signature. Mirrors `MIN_PEX_INTERVAL_MS`. (A bytes-budget / chunked
/// anti-entropy for large blobs is deferred to the fileshare slice — see
/// `MAX_CATCHUP_RESPONSE`.)
const MIN_BLOB_INTERVAL_MS: u64 = 200;
/// Cap on peer records returned in one PEX response / retained locally.
const MAX_PEX_ENTRIES: usize = 64;
/// Cap on dialable addresses carried per peer record.
const MAX_PEX_ADDRESSES: usize = 8;
/// Minimum interval (ms, on the injected clock) between PEX responses served to the
/// same requesting **member** — a rate limit so PEX cannot be used to amplify traffic.
const MIN_PEX_INTERVAL_MS: u64 = 1_000;
/// Cap on a single dialable address string in a peer record. Any real multiaddr is
/// far shorter; rejecting longer ones bounds the bytes a record can carry.
const MAX_PEX_ADDR_LEN: usize = 256;
/// Tight ceiling on a PEX **response** accepted from a serving member. A response is
/// at most `MAX_PEX_ENTRIES` records, each bounded by `MAX_PEX_ADDRESSES` ×
/// `MAX_PEX_ADDR_LEN`; 512 KiB is generous headroom. Far smaller than the 16 MiB
/// catch-up ceiling — a member cannot make us decode/verify an arbitrarily large
/// bundle (the receive-side bound matching the serve-side `take(MAX_PEX_ENTRIES)`).
const MAX_PEX_RESPONSE: usize = 512 * 1024;
/// Join-response status byte: the admission was staged and is awaiting the
/// fork-resolution window; the Welcome is pushed later (see `KIND_WELCOME`).
const JOIN_PENDING: u8 = 0;
/// Join-response status byte: the Welcome (and signature) follow inline.
const JOIN_READY: u8 = 1;
/// Defensive cap on the number of routing secrets a join transfer may carry — the
/// store only ever holds the current label plus two grandfathered ones.
const MAX_ROUTING_SECRETS: usize = 8;
/// Control requests (join/catch-up) are small; reject anything larger up front.
const MAX_CONTROL_REQUEST: usize = 64 * 1024;
/// Ceiling on a catch-up **response** accepted from a (untrusted) serving peer,
/// before it is decoded — a serving peer is no more trusted than a requester, so
/// the response is bounded just like the request. A full document history can be
/// large; 16 MiB is a generous v1 ceiling (resumable chunked anti-entropy for
/// larger-than-this histories is deferred — see ARCHITECTURE §2.8).
const MAX_CATCHUP_RESPONSE: usize = 16 * 1024 * 1024;
/// Defensive cap on the element count claimed by a bundle header, so a malformed
/// length prefix cannot drive a long allocation loop even within the size cap.
const MAX_BUNDLE_ELEMENTS: u32 = 1 << 20;
/// Byte budget for a catch-up **response** a node *generates*, mirroring the
/// inbound `MAX_CATCHUP_RESPONSE` cap so the serving side is hard-bounded too: a
/// peer cannot make us encode an arbitrarily large bundle. Over-budget responses
/// are served as a contiguous prefix; the requester pages the remainder.
const MAX_CONTROL_RESPONSE: usize = 16 * 1024 * 1024;
/// Freshness window for an authenticated catch-up request: the requester's signed
/// timestamp must be within this of the server's clock. Bounds replay of a
/// captured signed request (which, over the Noise transport, is already confined
/// to the server's own session).
const MAX_REQUEST_AGE_MS: u64 = 60_000;
/// Domain separator for the requester's proof-of-membership over a catch-up request.
const CATCHUP_AUTH_DOMAIN: &str = "catcoms/catchup-auth/v1";
/// Domain separator for the admitter's signature over a join response.
const JOIN_RESP_DOMAIN: &str = "catcoms/join-resp/v1";
/// Domain separator for a **responder's** signature over a commit-catch-up response,
/// proving the served bundle came from a current member (6e-3d-5). Binds the bundle
/// to the requester's key + the request timestamp so it cannot be replayed.
const CATCHUP_RESP_DOMAIN: &str = "catcoms/catchup-resp/v1";
/// Domain separator for a **responder's** signature over a PEX response bundle
/// (6e-3d-7) — same shape as the commit-catch-up response binding, distinct domain.
const PEX_RESP_DOMAIN: &str = "catcoms/pex-resp/v1";
/// Domain separator for a peer's **self-signature** over its own peer record (a
/// member binds its dialable addresses + seq to its device key), so a PEX responder
/// can only relay records peers signed themselves — it cannot forge a peer's address.
const PEER_RECORD_DOMAIN: &str = "catcoms/peer-record/v1";
/// Domain separator for a **responder's** signature over a blob-fetch response (8l) —
/// same binding shape as the catch-up/PEX responses, distinct domain.
const BLOB_FETCH_RESP_DOMAIN: &str = "catcoms/blob-fetch-resp/v1";

/// Tunable bounds for the recovery/key-window machinery. Every field is a hard
/// cap on memory the node will spend on out-of-order recovery, so a peer cannot
/// make a node allocate without bound. The defaults suit a desktop node; tests
/// shrink them to exercise eviction.
#[derive(Debug, Clone, Copy)]
pub struct SyncConfig {
    /// How many epochs back to retain per-document channel keys, so an op sealed
    /// just before an epoch advance still opens without a network round-trip.
    pub max_past_epochs: u64,
    /// Cap on retained membership commits available to serve catch-up to peers.
    pub max_commit_log: usize,
    /// Cap on out-of-order commits buffered awaiting their predecessors.
    pub max_pending_commits: usize,
    /// Largest forward epoch gap we will buffer/chase — a DoS bound against a
    /// peer flooding far-future (forged) commit records.
    pub max_commit_gap: u64,
    /// Cap on recently-seen peers retained as catch-up sources.
    pub max_known_peers: usize,
    /// Cap on queued (deferred) catch-up tasks.
    pub max_catchup_queue: usize,
    /// Cap on queued control-topic broadcasts awaiting send.
    pub max_outbox: usize,
    /// How many leaf ranks above the designated committer may still author a
    /// membership commit (0 = strict single-committer, the synchronous fast path;
    /// ≥1 enables concurrent committers and the staged fork-resolution path).
    pub max_committer_rank: u32,
    /// How long (ms, on the injected clock) a node collects competing same-epoch
    /// commits before adopting the lowest-`commit_id` winner — the fork-resolution
    /// contest window. Only used when `max_committer_rank >= 1`.
    pub stage_decision_window_ms: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            max_past_epochs: 8,
            max_commit_log: 256,
            max_pending_commits: 256,
            max_commit_gap: 1024,
            max_known_peers: 64,
            max_catchup_queue: 256,
            max_outbox: 256,
            max_committer_rank: 0,
            stage_decision_window_ms: 250,
        }
    }
}

/// A fork-resolution contest for the current epoch: a node collects competing
/// same-base candidate commits during a bounded window, then adopts the lowest
/// `commit_id`. Built only when `max_committer_rank >= 1`.
/// Context for an admission we staged: how to finalize (or reject) the join once
/// the fork-resolution contest resolves. The Welcome is **provisional** — it is
/// delivered to the joiner only if our staged Add becomes the canonical winner.
#[derive(Debug)]
struct StagedJoin {
    /// The joining peer, to push the Welcome / rejection to.
    joiner: PeerId,
    /// The single-use invite nonce, consumed only on a winning merge.
    nonce: [u8; 16],
    /// The openmls Welcome for the joiner. The inviter signature over the join
    /// transcript is (re)computed at resolution time, where the routing transfer it
    /// must bind has been sealed (post-merge epoch) — not here at stage time.
    welcome: Vec<u8>,
}

/// Our own staged commit within a contest.
#[derive(Debug)]
struct MyStaged {
    /// `commit_id` of our staged commit (so we know whether we won).
    commit_id: [u8; 32],
    /// Present if the staged commit is an admission (then resolution pushes the
    /// Welcome on a win or a rejection on a loss).
    join: Option<StagedJoin>,
    /// Whether our staged commit removes a member — so a win rotates the routing
    /// secret (`ns_secret_L`) just like the inbound apply path does.
    removed: bool,
}

#[derive(Debug)]
struct PendingResolve {
    /// The contested epoch (our current epoch until it resolves). Candidates must
    /// share our epoch-state fingerprint (a same-base fork); a candidate built on a
    /// different base is a deeper divergence and is refused in `contest_commit`.
    epoch: u64,
    /// The lowest-`commit_id` candidate seen so far (the provisional winner).
    best: CommitRecord,
    /// When the window closes (on the injected clock).
    deadline_ms: u64,
    /// Our own staged commit if we are a participant — so on resolution we know
    /// whether we won (merge our pending commit) or lost (abort it and apply the
    /// winner). `None` for a pure applier.
    mine: Option<MyStaged>,
}

/// A snapshot of a [`ChannelSync`]'s internal counters and gauges. Returned by
/// [`ChannelSync::stats`] for diagnostics — useful to assert recovery behaviour
/// in tests and to surface "why didn't this converge?" detail when debugging a
/// live node without parsing the trace log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncStats {
    /// Membership commits successfully applied (advancing the epoch).
    pub commits_applied: u64,
    /// Out-of-order membership commits buffered awaiting a predecessor.
    pub commits_buffered: u64,
    /// Commit-catch-up bundles this node served to peers.
    pub commits_served: u64,
    /// Commit-catch-up requests this node issued to recover missed commits.
    pub commit_catchups_requested: u64,
    /// Live ops decrypted under the current epoch.
    pub ops_ingested: u64,
    /// Live ops recovered with a retained past-epoch key (crossed a boundary).
    pub ops_recovered_past_epoch: u64,
    /// Live ops dropped because they were sealed under a future epoch we have
    /// not yet reached (we then chase the missing commits).
    pub ops_dropped_future_epoch: u64,
    /// Live ops dropped because they were sealed under an epoch older than the
    /// retained key window (we then request a document catch-up).
    pub ops_dropped_old_epoch: u64,
    /// Document catch-up requests issued to recover ops we could not decrypt.
    pub doc_catchups_requested: u64,
    /// Inbound requests refused (unauthenticated / not a current member / stale).
    pub requests_rejected: u64,
    /// Fork contests resolved (a winner adopted by tie-break).
    pub forks_resolved: u64,
    /// Our own staged commit lost a fork tie-break and was aborted.
    pub forks_lost: u64,
    /// Candidate commits refused as a too-deep fork (base-fingerprint mismatch).
    pub forks_too_deep: u64,
    /// Gauge: past-epoch channel keys currently retained.
    pub past_keys_retained: usize,
    /// Gauge: membership commits currently buffered out of order.
    pub pending_commits: usize,
    /// Gauge: membership commits currently retained to serve catch-up.
    pub commit_log_len: usize,
    /// Gauge: untrusted catch-up **candidate** peers currently known.
    pub known_peers: usize,
    /// Gauge: **proven member** catch-up sources currently known (promoted via a
    /// verified signed catch-up — the trusted pool).
    pub member_peers: usize,
}

/// Deferred recovery work, performed on the next async drain in [`ChannelSync::run_once`]
/// (the handlers that detect a gap run synchronously while processing an event).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatchupTask {
    /// Fetch and replay membership commits from `from_epoch` onward.
    Commits { from_epoch: u64 },
    /// Fetch a document's history (to recover an op we could not decrypt).
    Doc { doc_type: DocType, doc_id: u128 },
}

/// The transcript the admitter signs (and the joiner verifies) to authenticate a
/// Welcome: binds the Welcome bytes to the specific invite (group + nonce).
fn join_transcript(
    group_id: &[u8],
    invite_nonce: &[u8; 16],
    welcome: &[u8],
    sealed_routing: &[u8],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(JOIN_RESP_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(invite_nonce).expect("16 fits");
    e.put_bytes(welcome).expect("welcome fits");
    // Bind the sealed routing transfer so a relay cannot strip or corrupt it to
    // force the joiner onto a wrong (local L=0) routing baseline.
    e.put_bytes(sealed_routing).expect("sealed routing fits");
    e.finish()
}

/// The transcript a requester signs to prove it is a current member when asking
/// for catch-up, binding the proof to the group, the request kind + body, the
/// requester's own key, a freshness timestamp, a **per-request nonce** (anti-replay,
/// closing the same-millisecond-`ts` collision window), and the requester's **epoch**
/// at request time (so a captured request cannot be replayed into a later state).
fn catchup_auth_transcript(
    group_id: &[u8],
    kind: u8,
    inner: &[u8],
    requester_pubkey: &[u8],
    timestamp_ms: u64,
    nonce: &[u8; 16],
    req_epoch: u64,
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(CATCHUP_AUTH_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_u16(kind as u16);
    e.put_bytes(inner).expect("inner fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(timestamp_ms);
    e.put_bytes(nonce).expect("16 fits");
    e.put_u64(req_epoch);
    e.finish()
}

/// A **responder's** signature transcript over a served bundle (commit catch-up or
/// PEX), binding it to a `domain`, the group, the requester's key, the request
/// timestamp, the request's **nonce** and **epoch** (so a captured response cannot be
/// replayed against a *different* request, even one issued in the same millisecond),
/// and the bundle bytes — so the requester can verify the bundle was served by a
/// member for *this exact* request.
fn signed_resp_transcript(
    domain: &str,
    group_id: &[u8],
    requester_pubkey: &[u8],
    request_ts_ms: u64,
    nonce: &[u8; 16],
    req_epoch: u64,
    bundle: &[u8],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(domain).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(request_ts_ms);
    e.put_bytes(nonce).expect("16 fits");
    e.put_u64(req_epoch);
    e.put_bytes(bundle).expect("bundle fits");
    e.finish()
}

/// The responder transcript for a commit-catch-up bundle (6e-3d-5/6).
fn catchup_resp_transcript(
    group_id: &[u8],
    requester_pubkey: &[u8],
    request_ts_ms: u64,
    nonce: &[u8; 16],
    req_epoch: u64,
    bundle: &[u8],
) -> Vec<u8> {
    signed_resp_transcript(
        CATCHUP_RESP_DOMAIN,
        group_id,
        requester_pubkey,
        request_ts_ms,
        nonce,
        req_epoch,
        bundle,
    )
}

/// The responder transcript for a PEX bundle (6e-3d-7).
fn pex_resp_transcript(
    group_id: &[u8],
    requester_pubkey: &[u8],
    request_ts_ms: u64,
    nonce: &[u8; 16],
    req_epoch: u64,
    bundle: &[u8],
) -> Vec<u8> {
    signed_resp_transcript(
        PEX_RESP_DOMAIN,
        group_id,
        requester_pubkey,
        request_ts_ms,
        nonce,
        req_epoch,
        bundle,
    )
}

/// The responder transcript for a blob-fetch response (8l). Same binding shape as the
/// catch-up/PEX responses (binds the blob to the requester's key + request), distinct
/// domain — so a response cannot be lifted into another protocol or replayed.
fn blob_fetch_resp_transcript(
    group_id: &[u8],
    requester_pubkey: &[u8],
    request_ts_ms: u64,
    nonce: &[u8; 16],
    req_epoch: u64,
    blob: &[u8],
) -> Vec<u8> {
    signed_resp_transcript(
        BLOB_FETCH_RESP_DOMAIN,
        group_id,
        requester_pubkey,
        request_ts_ms,
        nonce,
        req_epoch,
        blob,
    )
}

/// Encode a blob-fetch request inner body: just the 32-byte content address.
fn encode_blob_fetch_req(cid: &Cid) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(cid.as_bytes()).expect("32 fits");
    e.finish()
}

/// Decode a blob-fetch request inner body into its content address.
fn decode_blob_fetch_req(bytes: &[u8]) -> Result<Cid, SyncError> {
    let mut d = Decoder::new(bytes);
    let cid_bytes: [u8; 32] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok(Cid::from_bytes(cid_bytes))
}

/// Frame a signed commit-catch-up response: `responder_pubkey ‖ sig ‖ bundle`.
fn encode_signed_commit_resp(
    responder_pubkey: &[u8],
    signature: &[u8; 64],
    bundle: &[u8],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(responder_pubkey).expect("pubkey fits");
    e.put_bytes(signature).expect("64 fits");
    e.put_bytes(bundle).expect("bundle fits");
    e.finish()
}

/// A parsed signed commit-catch-up response: `(responder pubkey, signature, bundle)`.
type SignedCommitResp = (Vec<u8>, [u8; 64], Vec<u8>);

/// Parse a signed commit-catch-up response into `(responder pubkey, signature, bundle)`.
fn decode_signed_commit_resp(bytes: &[u8]) -> Result<SignedCommitResp, SyncError> {
    let mut d = Decoder::new(bytes);
    let pubkey = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let signature: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let bundle = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((pubkey, signature, bundle))
}

/// Frame an authenticated catch-up request body: `inner ‖ pubkey ‖ ts ‖ nonce ‖
/// req_epoch ‖ sig`.
fn encode_authed_request(
    inner: &[u8],
    requester_pubkey: &[u8],
    timestamp_ms: u64,
    nonce: &[u8; 16],
    req_epoch: u64,
    signature: &[u8; 64],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(inner).expect("inner fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(timestamp_ms);
    e.put_bytes(nonce).expect("16 fits");
    e.put_u64(req_epoch);
    e.put_bytes(signature).expect("64 fits");
    e.finish()
}

/// A parsed authenticated request: `(inner body, requester pubkey, timestamp, nonce,
/// req_epoch, signature)`.
type AuthedRequest = (Vec<u8>, Vec<u8>, u64, [u8; 16], u64, [u8; 64]);

/// Parse an authenticated catch-up request body into
/// `(inner, pubkey, ts, nonce, req_epoch, sig)`.
fn decode_authed_request(bytes: &[u8]) -> Result<AuthedRequest, SyncError> {
    let mut d = Decoder::new(bytes);
    let inner = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let pubkey = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let timestamp_ms = d.get_u64().map_err(|_| SyncError::Malformed)?;
    let nonce: [u8; 16] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let req_epoch = d.get_u64().map_err(|_| SyncError::Malformed)?;
    let signature: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((inner, pubkey, timestamp_ms, nonce, req_epoch, signature))
}

/// Metadata a requester retains about a signed catch-up request (and a serving
/// handler recovers from one), so a responder's signed reply can be bound to *this
/// exact* request: the freshness timestamp, a per-request random nonce, and the
/// requester's epoch at request time. The nonce + epoch close the same-millisecond
/// `ts`-collision replay window the 6e-3d-5 review flagged.
#[derive(Debug, Clone, Copy)]
struct RequestAuth {
    ts: u64,
    nonce: [u8; 16],
    epoch: u64,
}

/// Decoded join response: `(welcome, inviter signature, sealed routing transfer)`.
type JoinResp = (Vec<u8>, [u8; 64], Vec<u8>);
/// Decoded routing state: the label and the retained `(slot, ns_secret)` pairs.
type RoutingStateParts = (u64, Vec<(u64, [u8; 32])>);

fn encode_join_resp(welcome: &[u8], signature: &[u8; 64], sealed_routing: &[u8]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(welcome).expect("welcome fits");
    e.put_bytes(signature).expect("64 fits");
    // The routing-state transfer (slice 6e-3d-2): the admitting member's `ns_secret_L`
    // history, sealed under the shared post-join epoch key. Empty if absent.
    e.put_bytes(sealed_routing).expect("sealed routing fits");
    e.finish()
}

/// Wire-encode a [`SealedBlob`] (nonce ‖ ciphertext) for transport.
fn encode_sealed(blob: &SealedBlob) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(&blob.nonce).expect("nonce fits");
    e.put_bytes(&blob.ciphertext).expect("ciphertext fits");
    e.finish()
}

fn decode_sealed(bytes: &[u8]) -> Result<SealedBlob, SyncError> {
    let mut d = Decoder::new(bytes);
    let nonce: [u8; 24] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let ciphertext = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok(SealedBlob { nonce, ciphertext })
}

/// Encode the routing state to seal for a joiner: the label `L` followed by every
/// retained `(slot, ns_secret_slot)`.
fn encode_routing_state(label: u64, secrets: &[(u64, [u8; 32])]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u64(label);
    e.put_u32(secrets.len() as u32);
    for (slot, secret) in secrets {
        e.put_u64(*slot);
        e.put_bytes(secret).expect("32 fits");
    }
    e.finish()
}

fn decode_routing_state(bytes: &[u8]) -> Result<RoutingStateParts, SyncError> {
    let mut d = Decoder::new(bytes);
    let label = d.get_u64().map_err(|_| SyncError::Malformed)?;
    let count = d.get_u32().map_err(|_| SyncError::Malformed)? as usize;
    if count > MAX_ROUTING_SECRETS {
        return Err(SyncError::Malformed);
    }
    let mut secrets = Vec::with_capacity(count);
    for _ in 0..count {
        let slot = d.get_u64().map_err(|_| SyncError::Malformed)?;
        let secret: [u8; 32] = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        secrets.push((slot, secret));
    }
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((label, secrets))
}

/// The join-time transfer plaintext sealed under `routing_transfer_key`: the routing state
/// **and** the group's stable file-wrap key (Phase 9h), so a joiner derives the same topics
/// *and* can open group files. Framed `len-prefixed routing ‖ 32-byte key`.
fn encode_join_transfer(
    label: u64,
    secrets: &[(u64, [u8; 32])],
    file_wrap_key: &[u8; 32],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(&encode_routing_state(label, secrets))
        .expect("routing fits");
    e.put_bytes(file_wrap_key).expect("32 fits");
    e.finish()
}

#[allow(clippy::type_complexity)]
fn decode_join_transfer(bytes: &[u8]) -> Result<(u64, Vec<(u64, [u8; 32])>, [u8; 32]), SyncError> {
    let mut d = Decoder::new(bytes);
    let routing = d.get_bytes().map_err(|_| SyncError::Malformed)?;
    let (label, secrets) = decode_routing_state(routing)?;
    let fwk: [u8; 32] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((label, secrets, fwk))
}

/// The routing state (`L` + the retained `ns_secret_L` history) transferred from an
/// admitting member to a joiner during the join handshake, so the joiner derives the
/// **same** blinded topics and rendezvous namespaces as the rest of the group
/// (a fresh node cannot re-derive past removal-epoch secrets on its own). Returned
/// by [`request_join`]; consumed by [`ChannelSync::new_joined`]. It also carries the group's
/// stable file-wrap key (Phase 9h). An empty state (only an inviter-side seal error post-9h)
/// leaves the joiner on its local `L = 0` with no file key.
#[derive(Default)]
pub struct RoutingState {
    label: u64,
    secrets: Vec<(u64, Zeroizing<[u8; 32]>)>,
    /// The group's stable file-wrap key (Phase 9h), transferred to the joiner alongside the
    /// routing secrets. `None` for an absent/empty transfer (the joiner then cannot open
    /// group files until it obtains the key).
    file_wrap_key: Option<[u8; 32]>,
}

impl std::fmt::Debug for RoutingState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the secrets.
        f.debug_struct("RoutingState")
            .field("label", &self.label)
            .field("secrets", &self.secrets.len())
            .finish()
    }
}

/// Open a sealed routing transfer received in a join response: derive the shared
/// transfer key from the just-joined group (the joiner is now at the seal epoch),
/// unseal, and decode.
///
/// An **absent** transfer (empty — now only an inviter-side seal/key-derivation **error**,
/// since post-9h a normal transfer always carries the file-wrap key + L=0 routing secret)
/// yields an empty state, so the joiner keeps its local `L = 0` and **no** file key (it then
/// cannot share/open group files until it rejoins). A **present but unopenable** transfer is a
/// hard error, not a silent `L = 0`: the
/// inviter signature has already authenticated these bytes (see `finish_join`), so a
/// non-empty blob that fails to unseal/decode is a real version/encoding fault that
/// would otherwise mis-route the joiner onto a wrong baseline.
fn open_routing_transfer(
    group: &ServerGroup,
    device: &MlsDevice,
    sealed: &[u8],
) -> Result<RoutingState, SyncError> {
    if sealed.is_empty() {
        return Ok(RoutingState::default());
    }
    let key = group
        .routing_transfer_key(device)
        .map_err(|_| SyncError::JoinRejected)?;
    let blob = decode_sealed(sealed)?;
    let plaintext = unseal(&key, &blob).map(Zeroizing::new).map_err(|_| {
        tracing::warn!("authenticated routing transfer failed to unseal; rejecting join");
        SyncError::JoinRejected
    })?;
    let (label, secrets, file_wrap_key) = decode_join_transfer(&plaintext)?;
    Ok(RoutingState {
        label,
        secrets: secrets
            .into_iter()
            .map(|(slot, s)| (slot, Zeroizing::new(s)))
            .collect(),
        file_wrap_key: Some(file_wrap_key),
    })
}

/// The transcript a member signs to request a removal (binds the request to the
/// group, the target, the requester's key, and a freshness timestamp).
fn remove_req_transcript(
    group_id: &[u8],
    target: &[u8; 32],
    requester_pubkey: &[u8],
    timestamp_ms: u64,
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(REMOVE_REQ_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(target).expect("32 fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(timestamp_ms);
    e.finish()
}

/// Encode a signed remove request body: `target ‖ requester_pubkey ‖ ts ‖ sig`. Only the
/// committer ever *receives* a remove request now (removal is owner-only), so nothing in the
/// library encodes one — this is retained for the forged-request rejection test.
#[cfg(test)]
fn encode_remove_request(
    target: &[u8; 32],
    requester_pubkey: &[u8],
    timestamp_ms: u64,
    signature: &[u8; 64],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(target).expect("32 fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(timestamp_ms);
    e.put_bytes(signature).expect("64 fits");
    e.finish()
}

/// A parsed remove request: `(target, requester_pubkey, timestamp, signature)`.
type RemoveRequest = ([u8; 32], Vec<u8>, u64, [u8; 64]);

fn decode_remove_request(bytes: &[u8]) -> Result<RemoveRequest, SyncError> {
    let mut d = Decoder::new(bytes);
    let target: [u8; 32] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let pubkey = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let timestamp_ms = d.get_u64().map_err(|_| SyncError::Malformed)?;
    let signature: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((target, pubkey, timestamp_ms, signature))
}

// --- admin invites (Option C): Add-request + admit-result codecs ---------------------------
// These define the stable wire format; the request/relay flow that uses them lands in the next
// slice. `#[allow(dead_code)]` until then (removed when wired into serve_join/on_control).

/// The transcript an authorized inviter signs for an Add-request: binds the target group, the
/// invite nonce, the joiner KeyPackage (by hash, so a relay can't swap it under a valid sig),
/// the requester key, and a timestamp (freshness). Domain-separated.
#[allow(dead_code)]
fn add_req_transcript(
    group_id: &[u8],
    invite_nonce: &[u8; 16],
    kp_hash: &[u8; 32],
    requester_pubkey: &[u8],
    ts: u64,
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(ADD_REQ_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(invite_nonce).expect("nonce fits");
    e.put_bytes(kp_hash).expect("hash fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(ts);
    e.finish()
}

/// Encode a signed Add-request body: `invite ‖ key_package ‖ requester_pubkey ‖ ts ‖ sig`.
#[allow(dead_code)]
fn encode_add_request(
    invite: &[u8],
    key_package: &[u8],
    requester_pubkey: &[u8],
    ts: u64,
    signature: &[u8; 64],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(invite).expect("invite fits");
    e.put_bytes(key_package).expect("kp fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(ts);
    e.put_bytes(signature).expect("64 fits");
    e.finish()
}

/// (invite, key_package, requester_pubkey, ts, signature)
type AddRequest = (Vec<u8>, Vec<u8>, Vec<u8>, u64, [u8; 64]);

#[allow(dead_code)]
fn decode_add_request(bytes: &[u8]) -> Result<AddRequest, SyncError> {
    let mut d = Decoder::new(bytes);
    let invite = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let key_package = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let pubkey = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let ts = d.get_u64().map_err(|_| SyncError::Malformed)?;
    let signature: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((invite, key_package, pubkey, ts, signature))
}

/// Encode the owner→admin admit-result push: `invite_nonce ‖ welcome ‖ sealed_routing ‖
/// owner_sig`.
#[allow(dead_code)]
fn encode_admit_result(
    invite_nonce: &[u8; 16],
    welcome: &[u8],
    sealed_routing: &[u8],
    owner_sig: &[u8; 64],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(invite_nonce).expect("nonce fits");
    e.put_bytes(welcome).expect("welcome fits");
    e.put_bytes(sealed_routing).expect("routing fits");
    e.put_bytes(owner_sig).expect("64 fits");
    e.finish()
}

/// (invite_nonce, welcome, sealed_routing, owner_sig)
type AdmitResult = ([u8; 16], Vec<u8>, Vec<u8>, [u8; 64]);

#[allow(dead_code)]
fn decode_admit_result(bytes: &[u8]) -> Result<AdmitResult, SyncError> {
    let mut d = Decoder::new(bytes);
    let nonce: [u8; 16] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let welcome = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let sealed_routing = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let owner_sig: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((nonce, welcome, sealed_routing, owner_sig))
}

/// Encode the committer→joiner Welcome push payload (a winning staged admission):
/// `[JOIN_READY] ‖ welcome ‖ signature`.
fn encode_welcome_push(welcome: &[u8], signature: &[u8; 64], sealed_routing: &[u8]) -> Vec<u8> {
    let mut out = vec![JOIN_READY];
    out.extend_from_slice(&encode_join_resp(welcome, signature, sealed_routing));
    out
}

fn decode_join_resp(bytes: &[u8]) -> Result<JoinResp, SyncError> {
    let mut d = Decoder::new(bytes);
    let welcome = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let signature: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let sealed_routing = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((welcome, signature, sealed_routing))
}

/// Errors from channel synchronization.
#[derive(Debug, Error)]
pub enum SyncError {
    /// A transport-level error.
    #[error(transparent)]
    Transport(#[from] catcoms_rt::TransportError),
    /// A replication-layer error.
    #[error(transparent)]
    Repl(#[from] catcoms_replication::ReplError),
    /// An MLS-layer error.
    #[error(transparent)]
    Mls(#[from] catcoms_mls::MlsError),
    /// A blob-store error (content-addressed storage).
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// The requested document is not open here.
    #[error("document not open")]
    NoSuchDoc,
    /// A join request was rejected by the inviter.
    #[error("join request rejected")]
    JoinRejected,
    /// A sync message was malformed.
    #[error("malformed sync message")]
    Malformed,
    /// The caller is not authorized for this action (e.g. a non-owner attempting a removal).
    #[error("not authorized")]
    Unauthorized,
}

/// A member's **self-signed, dialable peer record**, exchanged via PEX (6e-3d-7). The
/// member binds its transport `peer_id` and dialable `addresses` (plus a monotonic
/// `seq` for freshness) to its **device key**, so a PEX responder can only relay
/// records peers signed themselves — it cannot forge a peer's address — and a
/// recipient can confirm the record describes a current group member (the signer's
/// device id is in the roster) before treating it as a dial candidate.
///
/// NOTE for the discovery bridge (6e-3d-9): `seq` is a per-**device** counter, and the
/// authenticated identity is `device_pubkey`. When turning records into
/// `catcoms-discovery` `Candidate`s, key the candidate (and its anti-replay seq) on the
/// **device id**, not the self-asserted `peer_id` — two records could claim the same
/// `peer_id`, so keying on `peer_id` would let one member pin another's freshness.
/// `peer_id`/`addresses` are the dial target only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerDescriptor {
    /// The member's device public key (roster lookup + self-signature verification).
    pub device_pubkey: Vec<u8>,
    /// Its transport peer id (the dial target).
    pub peer_id: [u8; 32],
    /// Dialable multiaddr strings (opaque here; capped by `MAX_PEX_ADDRESSES`).
    pub addresses: Vec<String>,
    /// Monotonic per-device sequence number (freshness; never a server TTL).
    pub seq: u64,
    /// The device's signature over the canonical payload.
    pub signature: [u8; 64],
}

/// The canonical bytes a peer signs over its own record (length-prefixed, so no two
/// distinct records collide).
fn peer_record_signing_payload(
    device_pubkey: &[u8],
    peer_id: &[u8; 32],
    addresses: &[String],
    seq: u64,
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(PEER_RECORD_DOMAIN).expect("label fits");
    e.put_bytes(device_pubkey).expect("pubkey fits");
    e.put_bytes(peer_id).expect("32 fits");
    e.put_u64(seq);
    e.put_u32(addresses.len() as u32);
    for a in addresses {
        e.put_str(a).expect("addr fits");
    }
    e.finish()
}

impl PeerDescriptor {
    /// Verify the record's self-signature: the embedded device key signed its
    /// `(peer_id, addresses, seq)`. (Membership — that the signer is in the roster —
    /// is checked separately by the ingesting node.)
    pub fn verify_self(&self) -> bool {
        let payload = peer_record_signing_payload(
            &self.device_pubkey,
            &self.peer_id,
            &self.addresses,
            self.seq,
        );
        verify_with_public_bytes(&self.device_pubkey, &payload, &self.signature)
    }

    fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(&self.device_pubkey).expect("pubkey fits");
        e.put_bytes(&self.peer_id).expect("32 fits");
        e.put_u64(self.seq);
        e.put_u32(self.addresses.len() as u32);
        for a in &self.addresses {
            e.put_str(a).expect("addr fits");
        }
        e.put_bytes(&self.signature).expect("64 fits");
        e.finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, SyncError> {
        let mut d = Decoder::new(bytes);
        let device_pubkey = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
        let peer_id: [u8; 32] = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        let seq = d.get_u64().map_err(|_| SyncError::Malformed)?;
        let count = d.get_u32().map_err(|_| SyncError::Malformed)? as usize;
        // Bound the address count up front so a forged length cannot drive a long loop.
        if count > MAX_PEX_ADDRESSES {
            return Err(SyncError::Malformed);
        }
        let mut addresses = Vec::with_capacity(count);
        for _ in 0..count {
            let addr = d.get_str().map_err(|_| SyncError::Malformed)?;
            // Bound each address string so a record cannot smuggle megabytes of bytes
            // (the count cap alone bounds the number, not the per-string length).
            if addr.len() > MAX_PEX_ADDR_LEN {
                return Err(SyncError::Malformed);
            }
            addresses.push(addr.to_string());
        }
        let signature: [u8; 64] = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        d.finish().map_err(|_| SyncError::Malformed)?;
        Ok(Self {
            device_pubkey,
            peer_id,
            addresses,
            seq,
            signature,
        })
    }
}

/// Extract the dialable peer addresses from a [`ChannelSync::snapshot`] blob **without a full
/// restore** (Phase 9g), so a reloading node can dial its last-known peers at transport
/// construction and reconnect on its own. Mirrors the snapshot framing
/// (`mls ‖ routing ‖ ledger ‖ docs ‖ commit_log ‖ peer_records`); kept next to the encoders
/// it must track. Returns an empty list (not an error) for a node that knew no peers.
pub fn peer_addrs_from_snapshot(snapshot: &[u8]) -> Result<Vec<String>, SyncError> {
    let bad = || SyncError::Malformed;
    let mut d = Decoder::new(snapshot);
    // Skip the leading length-prefixed sections: MLS, routing, ledger.
    for _ in 0..3 {
        d.get_bytes().map_err(|_| bad())?;
    }
    // Skip the docs and the commit log (both `u32 count ‖ len-prefixed entries`).
    for _ in 0..2 {
        let count = d.get_u32().map_err(|_| bad())?;
        for _ in 0..count {
            d.get_bytes().map_err(|_| bad())?;
        }
    }
    // Read the peer records and collect their addresses.
    let peer_count = d.get_u32().map_err(|_| bad())?;
    let mut addrs = Vec::new();
    for _ in 0..peer_count {
        let desc = PeerDescriptor::decode(d.get_bytes().map_err(|_| bad())?)?;
        addrs.extend(desc.addresses);
    }
    Ok(addrs)
}

/// Frame a PEX bundle: `u32 count ‖ len-prefixed PeerDescriptors`.
fn encode_pex_bundle(records: &[PeerDescriptor]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u32(records.len() as u32);
    for r in records {
        e.put_bytes(&r.encode()).expect("record fits");
    }
    e.finish()
}

fn decode_pex_bundle(bytes: &[u8]) -> Result<Vec<PeerDescriptor>, SyncError> {
    let mut d = Decoder::new(bytes);
    let count = d.get_u32().map_err(|_| SyncError::Malformed)?;
    // A legitimate PEX response carries at most MAX_PEX_ENTRIES records (the serve
    // side caps at exactly that). Reject a larger claim BEFORE the per-record decode
    // + signature-verify loop, so a hostile member cannot pack a giant bundle to
    // force ~count Ed25519 verifications on the requester (member-on-member CPU DoS).
    if count as usize > MAX_PEX_ENTRIES {
        return Err(SyncError::Malformed);
    }
    let mut out = Vec::new();
    for _ in 0..count {
        let raw = d.get_bytes().map_err(|_| SyncError::Malformed)?;
        out.push(PeerDescriptor::decode(raw)?);
    }
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok(out)
}

/// The blinded gossip topic for a document at routing label `slot`, **keyed** under
/// that label's routing secret (`ns_secret_slot`): `BLAKE3_keyed(ns_secret_slot,
/// "…/topic/v2" ‖ group_id ‖ slot ‖ type ‖ id)`. Keyed (not just group-id-hashed,
/// v1) so a non-member holding an invite — which carries `group_id` in the clear —
/// cannot compute it; binding `slot` rotates it on every member removal (§2.5).
fn channel_topic(
    ns_secret: &[u8; 32],
    group_id: &[u8],
    doc_type: DocType,
    doc_id: u128,
    slot: u64,
) -> Topic {
    // Canonical, length-prefixed preimage (the Encoder frames every variable field)
    // so no two distinct (group_id, slot, type, id) tuples can collide into one topic.
    let mut e = Encoder::new();
    e.put_str("catcoms/topic/v2").expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_u64(slot);
    e.put_u16(doc_type.tag());
    e.put_u128(doc_id);
    Topic::new(
        blake3::keyed_hash(ns_secret, &e.finish())
            .as_bytes()
            .to_vec(),
    )
}

/// The per-group **control** topic carrying membership commits at routing label
/// `slot`, keyed under `ns_secret_slot` (v3). Like the channel topic it is
/// member-only and rotates on removal; commit delivery across a rotation is handled
/// by publishing a removal commit on the pre-rotation topic and a grandfathered
/// subscription window.
fn control_topic(ns_secret: &[u8; 32], group_id: &[u8], slot: u64) -> Topic {
    let mut e = Encoder::new();
    e.put_str("catcoms/control/v3").expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_u64(slot);
    Topic::new(
        blake3::keyed_hash(ns_secret, &e.finish())
            .as_bytes()
            .to_vec(),
    )
}

/// Domain separator for the pre-dial member-verifiable registration tag (6e-3d-6).
const RZ_TAG_DOMAIN: &str = "catcoms/rz-tag/v1";

/// The member-only registration **tag** a discoverer recomputes (it holds
/// `ns_secret_L`) to confirm a discovered record was published by a real member at
/// routing label `slot`, *before* spending a dial on it. It binds the registrant's
/// libp2p `peer_id` and its signed record `seq` so:
///
/// - a **non-member** (no `ns_secret_L`) cannot forge one — a leaked/guessed-namespace
///   Sybil flood is one rejected hash, **no dial**;
/// - a **colluding rendezvous** cannot graft a real member's tag onto an injected
///   Sybil record — the Sybil's `peer_id` differs, so the bound tag will not verify; and
/// - a **removed** member's `L-1` tag is rejected by a discoverer who has applied the
///   removal (it derives the tag under the new `ns_secret_L`).
///
/// It rides as a registrant-signed synthetic address in the libp2p `PeerRecord`
/// (carried + verified end-to-end in 6e-3d-9; this is the cryptographic primitive).
fn routing_membership_tag(
    ns_secret: &[u8; 32],
    group_id: &[u8],
    rz_peer: &[u8],
    slot: u64,
    peer_id: &[u8],
    seq: u64,
) -> [u8; 16] {
    // Canonical, length-prefixed preimage: framing rz_peer and peer_id makes their
    // boundary unambiguous, so a colluding rendezvous cannot shift bytes between them
    // to graft a real member's tag onto a Sybil with a different peer id.
    let mut e = Encoder::new();
    e.put_str(RZ_TAG_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_u64(slot);
    e.put_bytes(rz_peer).expect("rz peer fits");
    e.put_bytes(peer_id).expect("peer id fits");
    e.put_u64(seq);
    let hash = blake3::keyed_hash(ns_secret, &e.finish());
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&hash.as_bytes()[..16]);
    tag
}

/// Constant-time equality for a 16-byte tag (no early-out on the first differing byte,
/// so a verifier leaks no timing signal about how much of a forged tag matched).
fn ct_eq_16(a: &[u8; 16], b: &[u8; 16]) -> bool {
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Domain separator for the pre-join rendezvous namespace.
const JOIN_NS_DOMAIN: &str = "catcoms/join-rz/v1";

/// The **pre-join** rendezvous namespace a joiner can compute from the invite alone
/// (6e-3d-9), so it can discover the inviter at a rendezvous **without a hard-coded
/// server address** — before it holds any group exporter secret. Keyed off the invite
/// nonce (stretched to 32 bytes), bound to the group and the specific rendezvous node:
///
/// `join_ns = "catcoms1-" ‖ hex(BLAKE3_keyed(derive_key(nonce),
///   "…/join-rz/v1" ‖ group_id ‖ rz_peer)[..20])`.
///
/// Its secrecy is bounded by the invite's — an invite-holder can compute it, which is
/// acceptable for a single-use bootstrap rendezvous point. The inviter registers under
/// it pre-join; once joined, the member switches to the secret, rotation-aware
/// [`ChannelSync::rendezvous_namespaces`]. Both sides derive the identical string.
pub fn join_namespace(group_id: &[u8], invite_nonce: &[u8; 16], rz_peer: &[u8]) -> String {
    // Stretch the 16-byte nonce to a 32-byte keying key (the "HKDF" of the design).
    let key = blake3::derive_key("catcoms/join-rz/hkdf/v1", invite_nonce);
    let mut e = Encoder::new();
    e.put_str(JOIN_NS_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(rz_peer).expect("rz peer fits");
    let hash = blake3::keyed_hash(&key, &e.finish());
    format!("catcoms1-{}", &hash.to_hex().as_str()[..40])
}

/// Control-topic envelope tags (first byte of every control message). New op kinds
/// (further proposals, revocations) land in later 6d-2 sub-blocks.
const CTRL_COMMIT: u8 = 0;
/// Control-topic tag: a member's signed request that the **designated committer**
/// remove a target (the single-serializer proposal model, 6d-2b). Only the
/// designated committer acts on it, so no concurrent commits arise.
const CTRL_REMOVE_REQUEST: u8 = 1;
/// Control-topic tag: an **authorized inviter (owner/admin)**'s signed request that the
/// designated committer admit a joiner via an invite (admin invites, Option C). Only the
/// committer (owner) acts on it, so admins never produce a commit → no fork. The owner returns
/// the Welcome to the requesting admin, who re-signs the join transcript and pushes it to the
/// joiner (so the joiner's verification is unchanged — see docs/design-admin-invites.md).
#[allow(dead_code)] // wired into serve_join/on_control in the next slice
const CTRL_ADD_REQUEST: u8 = 2;

/// Domain separator for the committer's per-commit authorization signature.
const COMMIT_AUTH_DOMAIN: &str = "catcoms/commit-auth/v1";
/// Domain separator for a member's signed remove request.
const REMOVE_REQ_DOMAIN: &str = "catcoms/remove-req/v1";
/// Domain separator for an authorized inviter's signed add request.
const ADD_REQ_DOMAIN: &str = "catcoms/add-req/v1";
/// DoS bound: the owner queues at most this many pending Add-requests (drop-oldest, deduped on
/// invite nonce) so a flood can't force unbounded MLS Adds or memory.
const MAX_ADD_REQUESTS: usize = 64;
/// How often an admin re-broadcasts a pending Add-request until the owner delivers the result
/// (driven off run_once events — notably the owner's reconnect — so an offline owner is caught
/// up when it returns).
const ADD_REQ_RETRY_MS: u64 = 2_000;
/// Admin-side cap on how long a single Add-request is driven (re-broadcast), regardless of the
/// invite's own (possibly far-future) expiry — bounds the `outgoing_add_requests` lifetime.
const MAX_ADD_REQUEST_LIFETIME_MS: u64 = 3_600_000;

/// A membership commit fanned out on the control topic so existing members apply
/// it and advance to the same epoch. `commit_epoch` is the epoch the commit was
/// built at (it advances the group to `commit_epoch + 1`) — the linearization key
/// for ordered replay during recovery.
///
/// `base_authenticator` is the committer's epoch-state fingerprint *before* the
/// commit ([`ServerGroup::epoch_authenticator_id`]); two records at the same epoch
/// with the same fingerprint are a same-base fork (resolvable by tie-break), a
/// different one means the branches diverged earlier. `committer_sig` is the
/// committer's MLS-leaf signature over the authorization transcript, so a recipient
/// can confirm an *authorized* member produced it (openmls still independently
/// authenticates the inner commit — this is authorization, not state auth).
#[derive(Debug, Clone)]
struct CommitRecord {
    group_id: Vec<u8>,
    commit_epoch: u64,
    committer_device: [u8; 32],
    mls_commit: Vec<u8>,
    base_authenticator: [u8; 32],
    committer_sig: [u8; 64],
}

/// The transcript a committer signs to authorize a membership commit.
fn commit_auth_transcript(
    group_id: &[u8],
    commit_epoch: u64,
    base_authenticator: &[u8; 32],
    committer_device: &[u8; 32],
    mls_commit: &[u8],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(COMMIT_AUTH_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_u64(commit_epoch);
    e.put_bytes(base_authenticator).expect("32 fits");
    e.put_bytes(committer_device).expect("32 fits");
    e.put_bytes(blake3::hash(mls_commit).as_bytes())
        .expect("32 fits");
    e.finish()
}

impl CommitRecord {
    /// A deterministic, content-addressed id used as the fork tie-break key
    /// (lowest wins). Built from the bytes every recipient already holds, with no
    /// clock or receive-order input, so all members compute the same value.
    fn commit_id(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/commit-id/v1");
        h.update(&self.group_id);
        h.update(&self.commit_epoch.to_be_bytes());
        h.update(&self.base_authenticator);
        h.update(&self.committer_device);
        h.update(&self.mls_commit);
        *h.finalize().as_bytes()
    }

    /// The bytes the committer signs / a verifier re-derives for this record.
    fn auth_transcript(&self) -> Vec<u8> {
        commit_auth_transcript(
            &self.group_id,
            self.commit_epoch,
            &self.base_authenticator,
            &self.committer_device,
            &self.mls_commit,
        )
    }

    fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(&self.group_id).expect("group id fits");
        e.put_u64(self.commit_epoch);
        e.put_bytes(&self.committer_device).expect("32 fits");
        e.put_bytes(&self.mls_commit).expect("commit fits");
        e.put_bytes(&self.base_authenticator).expect("32 fits");
        e.put_bytes(&self.committer_sig).expect("64 fits");
        e.finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, SyncError> {
        let mut d = Decoder::new(bytes);
        let group_id = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
        let commit_epoch = d.get_u64().map_err(|_| SyncError::Malformed)?;
        let committer_device: [u8; 32] = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        let mls_commit = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
        let base_authenticator: [u8; 32] = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        let committer_sig: [u8; 64] = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        d.finish().map_err(|_| SyncError::Malformed)?;
        Ok(Self {
            group_id,
            commit_epoch,
            committer_device,
            mls_commit,
            base_authenticator,
            committer_sig,
        })
    }
}

fn encode_catchup_req(doc_type: DocType, doc_id: u128) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u16(doc_type.tag());
    e.put_u128(doc_id);
    e.finish()
}

fn decode_catchup_req(bytes: &[u8]) -> Result<(DocType, u128), SyncError> {
    let mut d = Decoder::new(bytes);
    let tag = d.get_u16().map_err(|_| SyncError::Malformed)?;
    let doc_type = DocType::from_tag(tag).ok_or(SyncError::Malformed)?;
    let doc_id = d.get_u128().map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((doc_type, doc_id))
}

fn encode_commit_catchup_req(from_epoch: u64) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u64(from_epoch);
    e.finish()
}

fn decode_commit_catchup_req(bytes: &[u8]) -> Result<u64, SyncError> {
    let mut d = Decoder::new(bytes);
    let from_epoch = d.get_u64().map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok(from_epoch)
}

fn encode_commit_bundle(records: &[CommitRecord]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u32(records.len() as u32);
    for r in records {
        e.put_bytes(&r.encode()).expect("commit record fits");
    }
    e.finish()
}

fn decode_commit_bundle(bytes: &[u8]) -> Result<Vec<CommitRecord>, SyncError> {
    let mut d = Decoder::new(bytes);
    let count = d.get_u32().map_err(|_| SyncError::Malformed)?;
    if count > MAX_BUNDLE_ELEMENTS {
        return Err(SyncError::Malformed);
    }
    let mut out = Vec::new();
    for _ in 0..count {
        let raw = d.get_bytes().map_err(|_| SyncError::Malformed)?;
        out.push(CommitRecord::decode(raw)?);
    }
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok(out)
}

fn encode_bundle(ops: &[SealedOp]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u32(ops.len() as u32);
    for op in ops {
        e.put_bytes(&op.encode()).expect("op fits");
    }
    e.finish()
}

/// Encode a contiguous prefix of `ops` whose total size fits `budget`, returning
/// the encoded bundle and how many ops it covers. The prefix is dependency-safe
/// because `ops` are in log order (earlier ops first).
fn size_capped_ops(ops: &[SealedOp], budget: usize) -> (Vec<u8>, usize) {
    let mut taken: Vec<SealedOp> = Vec::new();
    let mut size = 4; // bundle count header
    for op in ops {
        let entry = op.encode().len() + 4;
        if size + entry > budget {
            break;
        }
        size += entry;
        taken.push(op.clone());
    }
    (encode_bundle(&taken), taken.len())
}

fn decode_bundle(bytes: &[u8]) -> Result<Vec<SealedOp>, SyncError> {
    let mut d = Decoder::new(bytes);
    let count = d.get_u32().map_err(|_| SyncError::Malformed)?;
    if count > MAX_BUNDLE_ELEMENTS {
        return Err(SyncError::Malformed);
    }
    let mut ops = Vec::new();
    for _ in 0..count {
        let raw = d.get_bytes().map_err(|_| SyncError::Malformed)?;
        ops.push(SealedOp::decode(raw)?);
    }
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok(ops)
}

fn encode_join_req(invite: &InviteToken, key_package: &[u8]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(&invite.encode()).expect("invite fits");
    e.put_bytes(key_package).expect("key package fits");
    e.finish()
}

fn decode_join_req(bytes: &[u8]) -> Result<(InviteToken, Vec<u8>), SyncError> {
    let mut d = Decoder::new(bytes);
    let invite_bytes = d.get_bytes().map_err(|_| SyncError::Malformed)?;
    let invite = InviteToken::decode(invite_bytes).map_err(|_| SyncError::Malformed)?;
    let key_package = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((invite, key_package))
}

/// Synchronizes a server's documents over a [`MeshTransport`], and admits new
/// members via single-use invites. Generic over the injected RNG `R`
/// (e.g. `OsCryptoRng` in production, a seeded CSPRNG in tests).
/// Owner-side: an authorized admin's Add-request accepted and awaiting the next admit drain.
struct PendingAdd {
    invite: InviteToken,
    kp_bytes: Vec<u8>,
    admin: PeerId,
}

/// Owner-side: an admit the owner already finalized, cached by invite nonce so a *retransmitted*
/// request re-delivers the same result instead of failing the (now-consumed) ledger check.
struct CachedAdmit {
    welcome: Vec<u8>,
    sealed_routing: Vec<u8>,
    owner_sig: [u8; 64],
}

/// Admin-side: an Add-request this node is driving to completion (re-broadcast until the owner
/// delivers the admit result, then re-signed + pushed to the joiner).
struct OutgoingAdd {
    invite: InviteToken,
    kp_bytes: Vec<u8>,
    joiner: PeerId,
    next_retry_ms: u64,
    expires_at_ms: u64,
}

/// (invite_encoded, kp_bytes, invite_nonce, group_id) — one Add-request to re-broadcast.
type Rebroadcast = (Vec<u8>, Vec<u8>, [u8; 16], Vec<u8>);

pub struct ChannelSync<T: MeshTransport, R: CryptoRngCore> {
    transport: T,
    group: ServerGroup,
    device: MlsDevice,
    rng: R,
    clock: Box<dyn Clock + Send>,
    ledger: InviteLedger,
    docs: HashMap<(DocType, u128), EncryptedDoc>,
    /// The **current** routing label's control topic (where this node publishes
    /// commits). Recomputed whenever the routing label changes.
    control_topic: Topic,
    /// All control topics this node currently accepts inbound (the current label
    /// plus grandfathered ones) — used to tell a control message from a doc op on
    /// the gossip path. A subset of `routing_subs`.
    control_topics: HashSet<Topic>,
    /// Every routing-derived topic (control + per-doc, across the grandfather
    /// window) this node is currently subscribed to, so a label rotation can
    /// subscribe the new topics and unsubscribe the ones that aged out.
    routing_subs: HashSet<Topic>,
    /// Whether the control topic has been subscribed (`subscribe_control`), so a
    /// resync re-subscribes it across a rotation.
    control_subscribed: bool,
    /// A routing-label rotation happened; re-sync subscriptions on the next tick
    /// (subscribing is async, rotation is not).
    needs_resync: bool,
    config: SyncConfig,
    /// Membership commits queued for the control topic (drained in async run_once).
    outbox: Vec<(Topic, Vec<u8>)>,
    /// Membership commits we have applied/produced, retained to serve catch-up to
    /// peers that missed them. Ordered by `commit_epoch` (insertion order).
    commit_log: VecDeque<CommitRecord>,
    /// Membership commits received ahead of our epoch (a gap), keyed by
    /// `commit_epoch`, buffered for ordered replay once the gap fills.
    pending_commits: BTreeMap<u64, CommitRecord>,
    /// Past-epoch channel keys, captured before each epoch advance, so an op
    /// sealed under a just-superseded epoch still opens. Bounded and zeroized on
    /// eviction. Keyed by `(doc_type, doc_id, epoch)`.
    past_keys: BTreeMap<(DocType, u128, u64), Zeroizing<[u8; 32]>>,
    /// Member-**removal** counter `L` — the routing label. Advanced (only) when a
    /// commit removes a member; unchanged by Adds/Updates/application posts. The
    /// blinded gossip topics and rendezvous namespaces derive from the routing
    /// secret snapshotted at the current `L`, so they rotate **only on removal**
    /// (ARCHITECTURE §2.5). NOTE: a member joining after removals must receive the
    /// label and the snapshots via the join/catch-up transfer (6e-3d-9) — a fresh
    /// node locally initialises `L = 0`, which is only correct for the founder until
    /// that transfer lands.
    routing_label: u64,
    /// The routing secret (`ns_secret_L`) snapshotted at each removal, retained for
    /// the current and two previous labels `{L-2, L-1, L}` so a member one or two
    /// removals behind is still discoverable during the transition. Zeroized on
    /// eviction. openmls only exports the *current* epoch's secret, so this history
    /// is captured at the post-removal epoch (a removed member can never export it).
    routing_secrets: BTreeMap<u64, Zeroizing<[u8; 32]>>,
    /// The group's **stable** file-wrap key (Phase 9h): minted random at founding, transferred
    /// to each joiner alongside the routing state, and never rotated — so a late joiner can
    /// open *all* files (a per-epoch key would lock it out of files sealed under past epochs,
    /// the same problem the routing transfer solves). Files seal a fresh per-file content key
    /// wrapped under this. Zeroized; persisted in the snapshot. All-zero until set.
    file_wrap_key: Zeroizing<[u8; 32]>,
    /// Recently-seen peers (from gossip/requests/connections) — UNTRUSTED catch-up
    /// **candidates**: a Noise handshake is not group membership, so these may be
    /// Sybils. Tried only as a fallback, and a junk/unsigned reply is rejected.
    known_peers: VecDeque<PeerId>,
    /// Peers that served a **signed** commit catch-up verifying against the roster —
    /// proven current members (6e-3d-5). Preferred as catch-up sources, so a flood of
    /// un-handshaked candidates cannot crowd out a known-good source (the Sybil-C1
    /// fix). Bounded by `max_known_peers`.
    member_peers: VecDeque<PeerId>,
    /// Recovery work to perform on the next async drain.
    catchup_queue: Vec<CatchupTask>,
    /// Peers that recently answered a commit catch-up without filling the gap;
    /// skipped when choosing the next catch-up source so one bad/stale peer can't
    /// dead-end recovery. Cleared once a catch-up makes progress.
    failed_catchup_peers: VecDeque<PeerId>,
    /// An in-progress fork-resolution contest (only when `max_committer_rank >= 1`).
    pending: Option<PendingResolve>,
    /// Provisional-Welcome (or rejection) pushes to deliver to joiners once a
    /// staged admission resolves: `(joiner, payload)` drained in `run_once`.
    welcome_outbox: Vec<(PeerId, Vec<u8>)>,
    /// Admin invites (Option C) — owner-side: accepted Add-requests awaiting admission (bounded
    /// `MAX_ADD_REQUESTS`, deduped on invite nonce).
    add_request_queue: VecDeque<PendingAdd>,
    /// Owner-side: finalized admit results by invite nonce, to re-deliver on a retransmit.
    admit_results: HashMap<[u8; 16], CachedAdmit>,
    /// Owner-side: `KIND_ADMIT_RESULT` pushes to deliver to requesting admins (drained in `run_once`).
    admit_result_outbox: Vec<(PeerId, Vec<u8>)>,
    /// Admin-side: Add-requests this node is driving to completion, keyed by invite nonce.
    outgoing_add_requests: HashMap<[u8; 16], OutgoingAdd>,
    /// **Owner-local authoritative** admin set (fingerprints), persisted in the snapshot. This is
    /// the security source of truth for the admission gate (`inviter_is_authorized`): a malicious
    /// member cannot write it, so the demoted-admin grant-replay residual is closed (THREAT-MODEL
    /// item 3). The owner publishes a signed *copy* into the `MemberRoles` doc for display only.
    /// Empty on non-owner nodes (they don't admit; they read the published copy for display).
    admin_roster: BTreeSet<String>,
    /// Generation counter for the owner's *published* roster copy (monotonic; persisted), so
    /// honest members converge deterministically. Not load-bearing for the local gate.
    roster_gen: u64,
    /// Known **member** peer records (this node's own + those learned via PEX), each
    /// self-signed by a current member. The discovery layer turns these into
    /// PEX-sourced dial candidates. Bounded by `MAX_PEX_ENTRIES`.
    peer_records: HashMap<DeviceId, PeerDescriptor>,
    /// Per-requesting-**device** timestamp of the last PEX response served, for the
    /// PEX rate limit (keyed on the authenticated requester identity, not the
    /// transport connection, so multiple connections cannot multiply the rate).
    /// Bounded by `max_known_peers`.
    pex_served_at: HashMap<DeviceId, u64>,
    /// Per-requesting-**device** timestamp of the last blob response served, for the blob
    /// rate limit (same keying + bounding as `pex_served_at`).
    blob_served_at: HashMap<DeviceId, u64>,
    /// Content-addressed blob store for binaries fetched/served over the mesh (8l) —
    /// avatars and, later, fileshare. An in-memory store by default; a persistent store
    /// can be injected later. Boxed (not a generic param) so it does not ripple through
    /// `Server<T, R>` and the actor.
    blobs: Box<dyn BlobStore + Send>,
    /// Diagnostic counters (see [`SyncStats`]).
    stats: SyncStats,
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Build a synchronizer over `transport` for this member's `group`/`device`.
    /// `clock` is used to check invite expiry when serving join requests.
    pub fn new(
        transport: T,
        group: ServerGroup,
        device: MlsDevice,
        mut rng: R,
        clock: Box<dyn Clock + Send>,
    ) -> Self {
        // Mint this group's stable file-wrap key (Phase 9h). A founder keeps it; a joiner
        // replaces it with the transferred one via `adopt_routing_state`.
        let mut file_wrap_key = Zeroizing::new([0u8; 32]);
        rng.fill_bytes(file_wrap_key.as_mut());
        let mut this = Self {
            transport,
            group,
            device,
            rng,
            clock,
            file_wrap_key,
            ledger: InviteLedger::new(),
            docs: HashMap::new(),
            // Placeholder; set from `ns_secret_L` by `capture_routing_secret` below.
            control_topic: Topic::new(Vec::<u8>::new()),
            control_topics: HashSet::new(),
            routing_subs: HashSet::new(),
            control_subscribed: false,
            needs_resync: false,
            config: SyncConfig::default(),
            outbox: Vec::new(),
            commit_log: VecDeque::new(),
            pending_commits: BTreeMap::new(),
            past_keys: BTreeMap::new(),
            routing_label: 0,
            routing_secrets: BTreeMap::new(),
            known_peers: VecDeque::new(),
            member_peers: VecDeque::new(),
            catchup_queue: Vec::new(),
            failed_catchup_peers: VecDeque::new(),
            pending: None,
            welcome_outbox: Vec::new(),
            add_request_queue: VecDeque::new(),
            admit_results: HashMap::new(),
            admit_result_outbox: Vec::new(),
            outgoing_add_requests: HashMap::new(),
            admin_roster: BTreeSet::new(),
            roster_gen: 0,
            peer_records: HashMap::new(),
            pex_served_at: HashMap::new(),
            blob_served_at: HashMap::new(),
            blobs: Box::new(MemoryBlobStore::new()),
            stats: SyncStats::default(),
        };
        // Seed the L=0 routing secret from the current epoch. Correct for the
        // founder; a member that joined an existing group instead adopts the
        // transferred routing state via `new_joined` (the locally-seeded value is
        // then replaced).
        this.capture_routing_secret();
        this
    }

    /// Serialize this synchronizer's **durable** state into one blob for disk persistence
    /// (Phase 9e): the MLS state ([`snapshot_server`]), every [`EncryptedDoc`], the routing
    /// label + secrets, the invite ledger, the retained commit log, and the known peer
    /// records. The transient state (transport, topics, subscription flags, caches,
    /// rate-limit maps, blob store) is rebuilt on [`ChannelSync::restore`]. **Secret** — it
    /// holds the signer private key, group secrets, routing secrets and plaintext document
    /// content; the registry seals it under the vault key before it touches disk (9f).
    pub fn snapshot(&mut self) -> Result<Zeroizing<Vec<u8>>, SyncError> {
        let mls = snapshot_server(&self.device, &self.group)?;
        // `routing` carries the live routing secrets in the clear — zeroize the intermediate
        // (the returned blob is already `Zeroizing`).
        let routing = {
            let secrets: Vec<(u64, [u8; 32])> = self
                .routing_secrets
                .iter()
                .map(|(l, s)| (*l, **s))
                .collect();
            Zeroizing::new(encode_routing_state(self.routing_label, &secrets))
        };
        let ledger = self.ledger.snapshot();

        let mut e = Encoder::new();
        let oversize = || SyncError::Malformed;
        e.put_bytes(&mls).map_err(|_| oversize())?;
        e.put_bytes(&routing).map_err(|_| oversize())?;
        e.put_bytes(&ledger).map_err(|_| oversize())?;
        e.put_u32(self.docs.len() as u32);
        for doc in self.docs.values_mut() {
            let snap = doc.snapshot()?;
            e.put_bytes(&snap).map_err(|_| oversize())?;
        }
        e.put_u32(self.commit_log.len() as u32);
        for rec in &self.commit_log {
            e.put_bytes(&rec.encode()).map_err(|_| oversize())?;
        }
        e.put_u32(self.peer_records.len() as u32);
        for desc in self.peer_records.values() {
            e.put_bytes(&desc.encode()).map_err(|_| oversize())?;
        }
        // 9h: the stable file-wrap key, appended after the peer records so
        // `peer_addrs_from_snapshot` (which stops after them) is unaffected.
        e.put_bytes(&*self.file_wrap_key).map_err(|_| oversize())?;
        // Item 3: the owner-local authoritative admin roster + its generation, appended last
        // (also after the peer records, so `peer_addrs_from_snapshot` is still unaffected). This
        // is the security source of truth for the admission gate, so it MUST persist.
        e.put_u64(self.roster_gen);
        e.put_u32(self.admin_roster.len() as u32);
        for fp in &self.admin_roster {
            e.put_str(fp).map_err(|_| oversize())?;
        }
        Ok(Zeroizing::new(e.finish()))
    }

    /// Reconstruct a synchronizer from a [`ChannelSync::snapshot`] blob plus a **fresh**
    /// transport/rng/clock (connections do not persist — the caller re-dials, Phase 9g). The
    /// MLS device + group, documents, routing state, ledger, commit log and peer records are
    /// restored; the node then subscribes + resyncs at runtime exactly like a fresh one.
    pub fn restore(
        snapshot: &[u8],
        transport: T,
        rng: R,
        clock: Box<dyn Clock + Send>,
    ) -> Result<Self, SyncError> {
        let bad = || SyncError::Malformed;
        let mut d = Decoder::new(snapshot);
        let mls = d.get_bytes().map_err(|_| bad())?.to_vec();
        let routing_bytes = d.get_bytes().map_err(|_| bad())?.to_vec();
        let ledger_bytes = d.get_bytes().map_err(|_| bad())?.to_vec();
        let doc_count = d.get_u32().map_err(|_| bad())?;
        let mut doc_snaps = Vec::new();
        for _ in 0..doc_count {
            doc_snaps.push(d.get_bytes().map_err(|_| bad())?.to_vec());
        }
        let commit_count = d.get_u32().map_err(|_| bad())?;
        let mut commit_log = VecDeque::new();
        for _ in 0..commit_count {
            commit_log.push_back(CommitRecord::decode(d.get_bytes().map_err(|_| bad())?)?);
        }
        let peer_count = d.get_u32().map_err(|_| bad())?;
        let mut peer_records = HashMap::new();
        for _ in 0..peer_count {
            let desc = PeerDescriptor::decode(d.get_bytes().map_err(|_| bad())?)?;
            let id = DeviceId::from_public_key_bytes(&desc.device_pubkey);
            peer_records.insert(id, desc);
        }
        let file_wrap_key: [u8; 32] = d
            .get_bytes()
            .map_err(|_| bad())?
            .try_into()
            .map_err(|_| bad())?;
        // Item 3: the owner-local authoritative admin roster + generation. Read gracefully so a
        // pre-item-3 snapshot (no trailing bytes) loads with an empty roster (the owner re-grants;
        // old per-fp grant docs are no longer honored). A new snapshot is decoded strictly.
        let (roster_gen, admin_roster) = if d.is_empty() {
            (0u64, BTreeSet::new())
        } else {
            let gen = d.get_u64().map_err(|_| bad())?;
            let count = d.get_u32().map_err(|_| bad())?;
            let mut set = BTreeSet::new();
            for _ in 0..count {
                set.insert(d.get_str().map_err(|_| bad())?.to_string());
            }
            (gen, set)
        };
        d.finish().map_err(|_| bad())?;

        // Reconstruct the MLS device + group, then build a base synchronizer and override its
        // durable state. `new` re-derives an L=0 routing secret from the (post-restore) epoch;
        // `adopt_routing_state` then replaces it with the persisted label + secrets. The
        // file-wrap key is persisted separately (appended last), so the routing struct here
        // carries `None` for it.
        let (device, group) = restore_server(&mls)?;
        let mut this = Self::new(transport, group, device, rng, clock);
        let (label, secrets) = decode_routing_state(&routing_bytes)?;
        this.adopt_routing_state(RoutingState {
            label,
            secrets: secrets
                .into_iter()
                .map(|(l, s)| (l, Zeroizing::new(s)))
                .collect(),
            file_wrap_key: None,
        });
        this.file_wrap_key = Zeroizing::new(file_wrap_key);
        this.ledger = InviteLedger::restore(&ledger_bytes).map_err(|_| SyncError::Malformed)?;
        for snap in &doc_snaps {
            let doc = EncryptedDoc::restore(snap)?;
            this.docs.insert((doc.doc_type(), doc.doc_id()), doc);
        }
        this.commit_log = commit_log;
        this.peer_records = peer_records;
        this.roster_gen = roster_gen;
        this.admin_roster = admin_roster;
        Ok(this)
    }

    /// This node's own device id (for the product layer to re-derive its identity after a
    /// [`ChannelSync::restore`]).
    pub fn device_id(&self) -> DeviceId {
        self.device.device_id()
    }

    /// The designated committer's device id (the server owner — the MLS-anchored ownership
    /// the product layer uses, Phase 10h), if the group has one.
    pub fn designated_committer_id(&self) -> Option<DeviceId> {
        self.group.designated_committer()
    }

    /// Whether `device_id` is currently authorized to invite/admit — the **owner** (designated
    /// committer) unconditionally, or a current admin. On the **owner** (the only node that runs
    /// admission in Option C) this reads the owner's **local authoritative roster**, which a
    /// malicious member cannot write — so a demoted admin replaying/deleting its grant in the
    /// shared CRDT cannot re-authorize itself (THREAT-MODEL item 3). On a non-owner node it is a
    /// display fallback only (never the admission path), reading the owner-signed published roster.
    /// Returns `false` for a non-admin / non-member.
    pub fn inviter_is_authorized(&self, device_id: &DeviceId) -> bool {
        let Some(owner) = self.group.designated_committer() else {
            return false;
        };
        if owner == *device_id {
            return true;
        }
        let fp = roles::fingerprint(device_id);
        if self.is_designated_committer() {
            // We are the owner — the only node that runs admission (Option C). Consult our LOCAL
            // authoritative roster: replay/deletion/forgery against the shared CRDT cannot touch
            // it, so a demoted admin cannot re-authorize itself (THREAT-MODEL item 3).
            return self.admin_roster.contains(&fp);
        }
        // Non-owner: this is display/fallback only and does not gate admission. Trust the
        // owner-signed published roster (a stale replay here is cosmetic, never an admission).
        match self.doc(DocType::MemberRoles, roles::ROLES_DOC) {
            Some(doc) => roles::read_published_roster(doc.doc(), &self.group.group_id(), &owner)
                .is_some_and(|s| s.contains(&fp)),
            None => false,
        }
    }

    /// The owner's local authoritative admin set (fingerprints). Owner-only meaningful — empty on
    /// non-owner nodes. The product layer uses this for the owner's own role display so it always
    /// matches the admission gate (other members read the published copy).
    pub fn admin_roster(&self) -> HashSet<String> {
        self.admin_roster.iter().cloned().collect()
    }

    /// Owner-only: grant/revoke admin for `fp`, updating the **local authoritative** roster (the
    /// admission source of truth) and publishing a fresh owner-signed copy into the `MemberRoles`
    /// doc for display. The local set is updated first, so a later delete/replay of the published
    /// copy by a malicious member cannot affect the gate. Errors `Unauthorized` if not the owner;
    /// `NoSuchDoc` if the roles doc isn't open.
    pub async fn set_admin(&mut self, fp: &str, admin: bool) -> Result<(), SyncError> {
        if !self.is_designated_committer() {
            return Err(SyncError::Unauthorized);
        }
        if admin {
            self.admin_roster.insert(fp.to_string());
        } else {
            self.admin_roster.remove(fp);
        }
        self.publish_roster().await
    }

    /// Sign + publish the current `admin_roster` as the owner-signed `roster` value (display copy).
    /// Bumps `roster_gen`. Owner-only (callers are owner-gated); fails `NoSuchDoc` if the roles
    /// doc isn't open.
    async fn publish_roster(&mut self) -> Result<(), SyncError> {
        use automerge::{transaction::Transactable, ScalarValue, ROOT};
        self.roster_gen = self.roster_gen.saturating_add(1);
        let gen = self.roster_gen;
        let group_id = self.group.group_id();
        let fps: Vec<String> = self.admin_roster.iter().cloned().collect(); // BTreeSet ⇒ sorted
        let payload = roles::roster_payload(&group_id, gen, &fps);
        let sig = self.device.sign(&payload)?;
        let owner_pk = self.device.public_key_bytes();
        let value = roles::encode_roster(gen, &owner_pk, &fps, &sig);
        self.post(DocType::MemberRoles, roles::ROLES_DOC, move |d| {
            d.put(ROOT, roles::ROSTER_KEY, ScalarValue::Bytes(value))?;
            Ok(())
        })
        .await
    }

    /// Sign a blob with this device's signature key (for owner-signed capability records like
    /// role grants, Phase 10h). Verify with [`catcoms_crypto::verify_with_public_bytes`].
    pub fn sign_blob(&self, payload: &[u8]) -> Result<[u8; 64], SyncError> {
        Ok(self.device.sign(payload)?)
    }

    /// This device's signature public key bytes (to embed in a signed record + recompute the
    /// signer's device id on the verifying side).
    pub fn my_public_key(&self) -> Vec<u8> {
        self.device.public_key_bytes()
    }

    /// This server's MLS group id (stable across restarts) — used to key the on-disk blob
    /// store directory, so a reloaded server finds its sealed blobs.
    pub fn group_id(&self) -> Vec<u8> {
        self.group.group_id()
    }

    /// Replace the blob store (default in-memory) — e.g. with a persistent, sealing on-disk
    /// store (Phase 9h). Inject this right after construction, before any blob is added.
    pub fn set_blob_store(&mut self, blobs: Box<dyn BlobStore + Send>) {
        self.blobs = blobs;
    }

    /// Encrypt a file under this group's stable file-wrap key (Phase 9h). Returns its
    /// [`FileRef`] (to record in the encrypted file index) and the ciphertext blob to store +
    /// share over the mesh. The ciphertext is content-addressed by its own CID; only members
    /// holding the group file-wrap key can open it (end-to-end, not just at rest).
    pub fn seal_file(
        &mut self,
        plaintext: &[u8],
        mime: &str,
    ) -> Result<(FileRef, Vec<u8>), SyncError> {
        Ok(seal_file_fn(
            plaintext,
            mime,
            &self.file_wrap_key,
            &mut self.rng,
        )?)
    }

    /// Open a ciphertext blob produced by [`ChannelSync::seal_file`], given its [`FileRef`].
    pub fn open_file(&self, stored: &[u8], file_ref: &FileRef) -> Result<Vec<u8>, SyncError> {
        Ok(open_file_fn(stored, file_ref, &self.file_wrap_key)?)
    }

    /// Whether this node holds a (non-zero) group file-wrap key yet (a joiner has it only
    /// after adopting the join transfer).
    pub fn has_file_key(&self) -> bool {
        *self.file_wrap_key != [0u8; 32]
    }

    /// Build a synchronizer for a member that **joined** an existing group, adopting
    /// the routing state ([`RoutingState`]) transferred in the join response so it
    /// derives the same blinded topics and rendezvous namespaces as the group. Use
    /// this (not [`ChannelSync::new`]) for the `ServerGroup` returned by
    /// [`request_join`]; an empty transfer falls back to a local `L = 0`.
    pub fn new_joined(
        transport: T,
        group: ServerGroup,
        device: MlsDevice,
        rng: R,
        clock: Box<dyn Clock + Send>,
        routing: RoutingState,
    ) -> Self {
        let mut this = Self::new(transport, group, device, rng, clock);
        // A joiner has NO group file-wrap key of its own — only the founder mints one. Zero
        // the random key `new` seeded so that an absent/failed transfer leaves `has_file_key`
        // false (and `add_file` refuses), rather than a wrong random key that would silently
        // seal files no other member could open. A successful transfer installs the real key.
        this.file_wrap_key = Zeroizing::new([0u8; 32]);
        this.adopt_routing_state(routing);
        this
    }

    /// Override the recovery/key-window bounds (defaults to [`SyncConfig::default`]).
    /// Useful for tests and for tuning a constrained device.
    pub fn set_config(&mut self, config: SyncConfig) {
        self.config = config;
    }

    /// Subscribe to this group's control topic so the member receives membership
    /// commits. Call once after construction (and after joining).
    pub async fn subscribe_control(&mut self) -> Result<(), SyncError> {
        self.control_subscribed = true;
        self.resync_subscriptions().await
    }

    /// The current epoch (for tests / diagnostics).
    pub fn epoch(&self) -> u64 {
        self.group.epoch()
    }

    /// A snapshot of diagnostic counters and gauges for this node.
    pub fn stats(&self) -> SyncStats {
        let mut s = self.stats.clone();
        s.past_keys_retained = self.past_keys.len();
        s.pending_commits = self.pending_commits.len();
        s.commit_log_len = self.commit_log.len();
        s.known_peers = self.known_peers.len();
        s.member_peers = self.member_peers.len();
        s
    }

    /// Mint a single-use, device-bound invite to this server (the inviting device
    /// is this member). Record it so this node can later admit the joiner.
    pub fn mint_invite(
        &self,
        invite_nonce: [u8; 16],
        expires_at_ms: u64,
        bootstrap: Vec<String>,
    ) -> Result<InviteToken, SyncError> {
        Ok(self
            .group
            .mint_invite(&self.device, invite_nonce, expires_at_ms, bootstrap)?)
    }

    /// Mint an invite that also carries zero-knowledge **rendezvous** infra addresses
    /// (6e-3d-9), so a joiner can discover this server under the pre-join
    /// [`join_namespace`] without a hard-coded address. The rendezvous set is bound
    /// into the inviter signature.
    pub fn mint_invite_with_rendezvous(
        &self,
        invite_nonce: [u8; 16],
        expires_at_ms: u64,
        bootstrap: Vec<String>,
        rendezvous: Vec<String>,
    ) -> Result<InviteToken, SyncError> {
        Ok(self.group.mint_invite_with_rendezvous(
            &self.device,
            invite_nonce,
            expires_at_ms,
            bootstrap,
            rendezvous,
        )?)
    }

    /// Open a document: create it locally (if absent) and subscribe to its topic.
    pub async fn open_channel(&mut self, doc_type: DocType, doc_id: u128) -> Result<(), SyncError> {
        let key = (doc_type, doc_id);
        let actor = self.device.device_id();
        self.docs
            .entry(key)
            .or_insert_with(|| EncryptedDoc::new(doc_type, doc_id, &actor));
        tracing::info!(
            ?doc_type,
            doc_id,
            epoch = self.group.epoch(),
            "open channel"
        );
        // Subscribe this doc's topic(s) across the grandfather window.
        self.resync_subscriptions().await
    }

    /// Apply a local edit to a document and broadcast the resulting sealed op.
    pub async fn post<F>(
        &mut self,
        doc_type: DocType,
        doc_id: u128,
        edit: F,
    ) -> Result<(), SyncError>
    where
        F: FnOnce(&mut AutoCommit) -> Result<(), AutomergeError>,
    {
        let key = (doc_type, doc_id);
        let doc = self.docs.get_mut(&key).ok_or(SyncError::NoSuchDoc)?;
        let sealed = doc.edit(&self.device, &self.group, &mut self.rng, edit)?;
        // Publish on the current routing label's channel topic.
        let topic = self
            .channel_topic_for(doc_type, doc_id, self.routing_label)
            .ok_or(SyncError::NoSuchDoc)?;
        let bytes = sealed.encode();
        tracing::debug!(
            ?doc_type,
            doc_id,
            epoch = sealed.epoch,
            bytes = bytes.len(),
            "post op"
        );
        self.transport.publish(topic, Bytes::from(bytes)).await?;
        Ok(())
    }

    /// Process one inbound transport event (gossiped op, membership commit, or a
    /// catch-up / join request), after first draining queued broadcasts and any
    /// pending recovery work. Returns `false` when the transport has closed.
    pub async fn run_once(&mut self) -> Result<bool, SyncError> {
        // Admin invites (Option C): re-broadcast any pending Add-request whose retry elapsed
        // (caught up by the owner on its reconnect), then flush the Welcome a result produced —
        // the admin relays it to the joiner here (in single-committer mode the contest path that
        // normally drains the Welcome outbox never runs).
        self.drive_outgoing_add_requests();
        // Flush any queued membership-commit broadcasts + Add-request retransmits.
        self.drain_outbox().await;
        self.drain_welcome_outbox().await;
        // Owner: push finalized admit results to admins here too — a tick that admitted may be
        // cancelled (in a select!-driven loop) after publishing the commit but before sending the
        // result, so draining at the top each tick makes delivery robust.
        self.drain_admit_result_outbox().await;
        // Apply any routing-label rotation from a previous tick: subscribe the new
        // label's topics and drop the ones that aged out of the grandfather window.
        self.resync_if_needed().await;
        // Resolve a fork-resolution contest whose window has closed FIRST (local
        // work) — ahead of recovery, so a tick spent on catch-up can never stretch
        // the contest window (I4), then deliver any provisional Welcome it produced.
        if self.resolve_pending_if_expired() {
            self.drain_welcome_outbox().await;
            return Ok(true);
        }
        // Perform recovery work queued by the previous event (commit/doc catch-up).
        // A tick that spent its turn fetching catch-up yields here instead of
        // blocking on a fresh event — the recovery *was* this tick's work.
        if self.drain_catchup_queue().await {
            return Ok(true);
        }

        let event = self.transport.next_event().await;
        match event {
            None => Ok(false),
            Some(TransportEvent::Gossip { topic, from, data }) => {
                self.remember_peer(from);
                if self.control_topics.contains(&topic) {
                    self.on_control(from, &data);
                    // The designated committer may have queued a commit in response to a remove
                    // request, or an admin Add-request (Option C) — admit it + fan out the commit
                    // here; the result push to the admin happens at the top of the next tick
                    // (robust against this tick being cancelled mid-flight).
                    self.drain_add_request_queue();
                    self.drain_outbox().await;
                    self.resync_if_needed().await;
                } else {
                    self.on_gossip(&data);
                }
                Ok(true)
            }
            Some(TransportEvent::Request {
                from,
                data,
                responder,
                ..
            }) => {
                self.remember_peer(from);
                let response = self.handle_request(from, &data);
                // Broadcast any membership commit produced by serving the request
                // BEFORE telling the joiner it succeeded, so a crash leaves the
                // joiner to retry rather than the group silently missing the Add.
                self.drain_outbox().await;
                responder.respond(Bytes::from(response));
                Ok(true)
            }
            Some(TransportEvent::PeerConnected(peer)) => {
                self.remember_peer(peer);
                // A freshly-connected peer is a catch-up source; proactively probe in
                // case we fell behind while the live topic was outside our window
                // (commit catch-up is point-to-point, so it works off-topic). Deduped,
                // and skipped on the committer (it never lags and must keep serving).
                self.maybe_probe_for_missed_commits();
                Ok(true)
            }
            Some(_) => Ok(true),
        }
    }

    /// Publish all queued control-topic broadcasts; re-queue any that fail, bounded
    /// by `max_outbox` (drop oldest on overflow so a persistently failing transport
    /// cannot grow the queue without bound).
    async fn drain_outbox(&mut self) {
        let pending = std::mem::take(&mut self.outbox);
        for (topic, bytes) in pending {
            if self
                .transport
                .publish(topic.clone(), Bytes::from(bytes.clone()))
                .await
                .is_err()
            {
                self.outbox.push((topic, bytes));
            }
        }
        while self.outbox.len() > self.config.max_outbox {
            self.outbox.remove(0);
        }
    }

    /// Push any resolved provisional-Welcome outcomes to their joiners over
    /// request/response (best-effort: if the joiner is unreachable the join just
    /// fails and the joiner retries).
    async fn drain_welcome_outbox(&mut self) {
        let pending = std::mem::take(&mut self.welcome_outbox);
        for (joiner, payload) in pending {
            let mut req = vec![KIND_WELCOME];
            req.extend_from_slice(&payload);
            let _ = self
                .transport
                .request(joiner, ProtocolId(RR_PROTOCOL), Bytes::from(req))
                .await;
        }
    }

    /// Perform any queued recovery (commit / document catch-up). Tasks queued
    /// while servicing this drain wait for the next `run_once`, so one tick does
    /// bounded work. Returns `true` if at least one catch-up request was attempted
    /// (so `run_once` can yield rather than block on a fresh event); a task with
    /// no known peer is re-queued and does not count.
    async fn drain_catchup_queue(&mut self) -> bool {
        let tasks = std::mem::take(&mut self.catchup_queue);
        let mut attempted = false;
        for task in tasks {
            let Some(peer) = self.pick_catchup_peer() else {
                // No usable catch-up source known yet; keep the task for a later
                // tick (a new peer may appear). If every known peer is currently
                // marked failed, that means we have tried them all without filling
                // the gap — stop chasing until a fresh peer is seen.
                if self.known_peers.is_empty() {
                    self.catchup_queue.push(task);
                }
                continue;
            };
            attempted = true;
            match task {
                CatchupTask::Commits { from_epoch } => {
                    let before = self.group.epoch();
                    let _ = self.do_commit_catchup(peer, from_epoch).await;
                    if self.pending_commits.is_empty() && self.group.epoch() > before {
                        // Progress made: clear the failed-peer set and stop chasing.
                        self.failed_catchup_peers.clear();
                    } else if !self.pending_commits.is_empty() {
                        // This peer did not fill the gap; exclude it and retry next
                        // tick with a different source.
                        self.note_failed_catchup_peer(peer);
                        let here = self.group.epoch();
                        self.enqueue_commit_catchup(here);
                    }
                }
                CatchupTask::Doc { doc_type, doc_id } => {
                    let _ = self.request_catchup(peer, doc_type, doc_id).await;
                }
            }
        }
        attempted
    }

    /// Remember `peer` as a catch-up source (most-recently-seen at the back),
    /// bounded by `max_known_peers`. A freshly-seen peer is also un-marked as
    /// failed, so it becomes eligible again.
    fn remember_peer(&mut self, peer: PeerId) {
        if let Some(pos) = self.known_peers.iter().position(|p| *p == peer) {
            self.known_peers.remove(pos);
        }
        self.known_peers.push_back(peer);
        while self.known_peers.len() > self.config.max_known_peers {
            let evicted = self.known_peers.pop_front();
            if let Some(p) = evicted {
                self.failed_catchup_peers.retain(|f| *f != p);
            }
        }
        self.failed_catchup_peers.retain(|f| *f != peer);
    }

    /// Pick a catch-up source: a **proven member** (verified via a signed catch-up)
    /// first, falling back to an untrusted candidate to bootstrap — so a flood of
    /// un-handshaked candidates cannot crowd out a known-good source. Skips peers that
    /// just failed to fill a gap.
    fn pick_catchup_peer(&self) -> Option<PeerId> {
        self.member_peers
            .iter()
            .rev()
            .find(|p| !self.failed_catchup_peers.contains(p))
            .or_else(|| {
                self.known_peers
                    .iter()
                    .rev()
                    .find(|p| !self.failed_catchup_peers.contains(p))
            })
            .copied()
    }

    /// Record `peer` as a proven current member (it served a signed catch-up that
    /// verified against the roster). Most-recent-wins, bounded by `max_known_peers`,
    /// and un-marked as failed so it is eligible again.
    fn promote_member_peer(&mut self, peer: PeerId) {
        self.failed_catchup_peers.retain(|p| *p != peer);
        if let Some(pos) = self.member_peers.iter().position(|p| *p == peer) {
            self.member_peers.remove(pos);
        }
        self.member_peers.push_back(peer);
        while self.member_peers.len() > self.config.max_known_peers {
            self.member_peers.pop_front();
        }
    }

    /// Remove `peer` from the trusted member pool — e.g. its response showed it is no
    /// longer in the roster (a removed member), so it must stop being preferred as a
    /// catch-up source.
    fn demote_member_peer(&mut self, peer: PeerId) {
        self.member_peers.retain(|p| *p != peer);
    }

    /// Mark `peer` as having answered a catch-up without filling the gap, so the
    /// next attempt prefers a different source. Bounded by `max_known_peers`.
    fn note_failed_catchup_peer(&mut self, peer: PeerId) {
        if !self.failed_catchup_peers.contains(&peer) {
            self.failed_catchup_peers.push_back(peer);
        }
        while self.failed_catchup_peers.len() > self.config.max_known_peers {
            self.failed_catchup_peers.pop_front();
        }
    }

    /// Queue a membership-commit catch-up from `from_epoch` (deduped: at most one
    /// commit catch-up is ever queued at a time; bounded by `max_catchup_queue`).
    fn enqueue_commit_catchup(&mut self, from_epoch: u64) {
        if self
            .catchup_queue
            .iter()
            .any(|t| matches!(t, CatchupTask::Commits { .. }))
        {
            return;
        }
        if self.catchup_queue.len() >= self.config.max_catchup_queue {
            tracing::warn!("catch-up queue full; dropping a commit catch-up task");
            return;
        }
        self.catchup_queue.push(CatchupTask::Commits { from_epoch });
    }

    /// Queue a document catch-up (deduped per document; bounded by `max_catchup_queue`).
    fn enqueue_doc_catchup(&mut self, doc_type: DocType, doc_id: u128) {
        let task = CatchupTask::Doc { doc_type, doc_id };
        if self.catchup_queue.contains(&task) {
            return;
        }
        if self.catchup_queue.len() >= self.config.max_catchup_queue {
            tracing::warn!("catch-up queue full; dropping a doc catch-up task");
            return;
        }
        self.catchup_queue.push(task);
    }

    /// Build an authenticated catch-up request: `[kind] ‖ inner ‖ pubkey ‖ ts ‖
    /// nonce ‖ req_epoch ‖ sig`, where the signature is this member's proof it is
    /// currently in the group. Returns the framed bytes and the request's
    /// [`RequestAuth`] (ts/nonce/epoch) so the caller can later verify a responder's
    /// signed reply is bound to this exact request (anti-replay).
    fn build_authed_request(
        &mut self,
        kind: u8,
        inner: &[u8],
    ) -> Result<(Vec<u8>, RequestAuth), SyncError> {
        let pubkey = self.device.public_key_bytes();
        let ts = self.clock.now_ms();
        let mut nonce = [0u8; 16];
        self.rng.fill_bytes(&mut nonce);
        let epoch = self.group.epoch();
        let transcript = catchup_auth_transcript(
            &self.group.group_id(),
            kind,
            inner,
            &pubkey,
            ts,
            &nonce,
            epoch,
        );
        let signature = self.device.sign(&transcript)?;
        let mut out = vec![kind];
        out.extend_from_slice(&encode_authed_request(
            inner, &pubkey, ts, &nonce, epoch, &signature,
        ));
        Ok((out, RequestAuth { ts, nonce, epoch }))
    }

    /// Verify an inbound authenticated catch-up request and return its inner body
    /// iff the requester proved current group membership with a fresh signature.
    /// Counts a rejection in [`SyncStats`]. `kind` is the matched request kind.
    /// Returns `(inner body, requester pubkey, [`RequestAuth`])` on success — the
    /// pubkey + auth metadata let a serving handler bind its signed reply to this
    /// exact request (the nonce + epoch close the same-millisecond replay window).
    fn authenticate_request(
        &mut self,
        kind: u8,
        data: &[u8],
    ) -> Option<(Vec<u8>, Vec<u8>, RequestAuth)> {
        let (inner, pubkey, ts, nonce, req_epoch, signature) = match decode_authed_request(data) {
            Ok(parts) => parts,
            Err(_) => {
                self.stats.requests_rejected += 1;
                return None;
            }
        };
        // The requester must be a current member of THIS group.
        let device_id = DeviceId::from_public_key_bytes(&pubkey);
        if !self.group.contains_device(&device_id) {
            tracing::warn!("catch-up request from a non-member; refused");
            self.stats.requests_rejected += 1;
            return None;
        }
        // Freshness: bound replay of a captured signed request.
        let now = self.clock.now_ms();
        if now.abs_diff(ts) > MAX_REQUEST_AGE_MS {
            tracing::warn!("catch-up request timestamp out of freshness window; refused");
            self.stats.requests_rejected += 1;
            return None;
        }
        let transcript = catchup_auth_transcript(
            &self.group.group_id(),
            kind,
            &inner,
            &pubkey,
            ts,
            &nonce,
            req_epoch,
        );
        if !verify_with_public_bytes(&pubkey, &transcript, &signature) {
            tracing::warn!("catch-up request signature invalid; refused");
            self.stats.requests_rejected += 1;
            return None;
        }
        Some((
            inner,
            pubkey,
            RequestAuth {
                ts,
                nonce,
                epoch: req_epoch,
            },
        ))
    }

    /// Snapshot the channel keys for every open document at the **current** epoch,
    /// so ops sealed under it still open after the next epoch advance. Call right
    /// before any operation that advances the epoch.
    fn snapshot_epoch_keys(&mut self) {
        let epoch = self.group.epoch();
        let docs: Vec<(DocType, u128)> = self.docs.keys().copied().collect();
        for (doc_type, doc_id) in docs {
            match self.group.channel_secret(&self.device, doc_type, doc_id) {
                Ok(secret) => {
                    self.past_keys
                        .insert((doc_type, doc_id, epoch), Zeroizing::new(secret));
                }
                Err(e) => {
                    tracing::warn!(error = %e, ?doc_type, doc_id, epoch, "could not snapshot epoch key");
                }
            }
        }
    }

    /// Drop (and zeroize) retained past-epoch keys older than the window.
    fn evict_past_keys(&mut self) {
        let cutoff = self
            .group
            .epoch()
            .saturating_sub(self.config.max_past_epochs);
        self.past_keys.retain(|(_, _, epoch), _| *epoch >= cutoff);
    }

    // --- routing label (ns_secret_L) ----------------------------------------
    //
    // The gossip topics and rendezvous namespaces derive from the routing secret
    // snapshotted at the current removal-count `L`. They therefore rotate ONLY on
    // member removal (ARCHITECTURE §2.5), not on every commit — even though the
    // underlying MLS exporter secret changes each epoch — because we read the
    // *snapshot* at `L`, not the live current secret.

    /// Snapshot the current-epoch routing secret into slot `routing_label`,
    /// retaining only `{L-2, L-1, L}` (older labels zeroized on drop). Called at
    /// construction (L=0) and after each removal advances `L`.
    fn capture_routing_secret(&mut self) {
        match self.group.routing_metadata_secret(&self.device) {
            Ok(secret) => {
                self.routing_secrets
                    .insert(self.routing_label, Zeroizing::new(secret));
            }
            Err(e) => {
                tracing::warn!(error = %e, label = self.routing_label, "could not snapshot routing secret");
            }
        }
        let cutoff = self.routing_label.saturating_sub(2);
        self.routing_secrets.retain(|label, _| *label >= cutoff);
        self.recompute_topics();
    }

    /// Recompute the cached control topic(s) from the current routing-label window.
    /// The current label drives `control_topic` (where we publish); the whole window
    /// drives `control_topics` (what we accept inbound). Called after any change to
    /// the routing label or secrets.
    fn recompute_topics(&mut self) {
        if let Some(current) = self.control_topic_for(self.routing_label) {
            self.control_topic = current;
        }
        self.control_topics = self
            .window_labels()
            .filter_map(|slot| self.control_topic_for(slot))
            .collect();
    }

    /// The routing labels currently in the grandfather window (those with a retained
    /// secret): the current label down to `L-2`.
    fn window_labels(&self) -> impl Iterator<Item = u64> {
        let lowest = self.routing_label.saturating_sub(2);
        (lowest..=self.routing_label).rev()
    }

    /// The control topic for routing label `slot`, or `None` if its secret is no
    /// longer retained.
    fn control_topic_for(&self, slot: u64) -> Option<Topic> {
        let secret = self.routing_secrets.get(&slot)?;
        Some(control_topic(secret, &self.group.group_id(), slot))
    }

    /// The channel topic for `(doc_type, doc_id)` at routing label `slot`.
    fn channel_topic_for(&self, doc_type: DocType, doc_id: u128, slot: u64) -> Option<Topic> {
        let secret = self.routing_secrets.get(&slot)?;
        Some(channel_topic(
            secret,
            &self.group.group_id(),
            doc_type,
            doc_id,
            slot,
        ))
    }

    /// Every routing-derived topic this node should be subscribed to: the control
    /// topics (if control was subscribed) and each open doc's topics, across the
    /// grandfather window — so a member up to two removals behind is still heard.
    fn desired_routing_topics(&self) -> HashSet<Topic> {
        let mut set = HashSet::new();
        for slot in self.window_labels() {
            if self.control_subscribed {
                if let Some(t) = self.control_topic_for(slot) {
                    set.insert(t);
                }
            }
            for (doc_type, doc_id) in self.docs.keys() {
                if let Some(t) = self.channel_topic_for(*doc_type, *doc_id, slot) {
                    set.insert(t);
                }
            }
        }
        set
    }

    /// Subscribe the routing topics that should now be subscribed and unsubscribe
    /// those that aged out of the window, so subscriptions track the current label.
    async fn resync_subscriptions(&mut self) -> Result<(), SyncError> {
        let desired = self.desired_routing_topics();
        for topic in desired.difference(&self.routing_subs) {
            self.transport.subscribe(topic.clone()).await?;
        }
        for topic in self.routing_subs.difference(&desired) {
            self.transport.unsubscribe(topic.clone()).await?;
        }
        self.routing_subs = desired;
        Ok(())
    }

    /// Enqueue a proactive, topic-independent commit catch-up unless this node is the
    /// designated committer. Rotation made topics label-specific, so a non-committer
    /// that has fallen behind no longer receives the live topic and would have no
    /// reactive recovery trigger; commit catch-up is point-to-point, so it recovers
    /// us regardless of how far behind we are (bounded by a serving peer's commit-log
    /// window). The committer is the serializer and never lags — and probing from it
    /// would stall its serve loop on a mid-join peer. Triggered on `PeerConnected`
    /// (the realistic recovery moment: startup / reconnect to a live peer); a
    /// continuously-connected node that silently falls behind is the documented
    /// residual, mitigated by point-to-point catch-up + the later discovery slices.
    fn maybe_probe_for_missed_commits(&mut self) {
        if !self.is_designated_committer() {
            self.enqueue_commit_catchup(self.group.epoch());
        }
    }

    /// Whether this node is the group's single designated committer (lowest leaf
    /// index) — the serializer that produces every commit and so never lags. Also the
    /// product layer's server "owner" anchor (Phase 10h).
    pub fn is_designated_committer(&self) -> bool {
        matches!(
            (
                self.group.member_leaf_index(&self.device.device_id()),
                self.group.designated_committer_index(),
            ),
            (Some(i), Some(c)) if i == c
        )
    }

    /// Apply a pending subscription resync flagged by a routing-label rotation,
    /// clearing the flag (and re-arming it if the transport call fails).
    async fn resync_if_needed(&mut self) {
        if !self.needs_resync {
            return;
        }
        self.needs_resync = false;
        if let Err(e) = self.resync_subscriptions().await {
            tracing::warn!(error = %e, "failed to resync subscriptions after rotation");
            self.needs_resync = true;
        }
    }

    /// Advance the routing label and snapshot the post-removal-epoch secret.
    /// Invoked once per applied commit that removed a member — on the local
    /// committer path and on every member's inbound apply path — so all members
    /// converge on the same `L` and the same `ns_secret_L`.
    fn rotate_routing_secret(&mut self) {
        self.routing_label += 1;
        self.capture_routing_secret(); // also recomputes the cached control topics
        self.needs_resync = true; // re-subscribe to the new label's topics next tick
        tracing::debug!(
            label = self.routing_label,
            "routing secret rotated (member removed)"
        );
    }

    /// React to an applied inbound commit: rotate the routing secret iff it removed
    /// a member. (The local committer path calls [`Self::rotate_routing_secret`]
    /// directly, since `commit_remove_now` is always a removal.)
    fn note_commit_applied(&mut self, incoming: &Incoming) {
        if matches!(incoming, Incoming::CommitApplied { removed: true }) {
            self.rotate_routing_secret();
        }
    }

    /// The current removal counter `L` (diagnostics / tests).
    pub fn routing_label(&self) -> u64 {
        self.routing_label
    }

    /// Derive the rendezvous namespace for this group at routing label `slot`, as
    /// seen by the rendezvous node `rz_peer` (its libp2p peer-id bytes). Returns
    /// `None` if that label's secret is no longer retained.
    ///
    /// `namespace = "catcoms1-" ‖ hex(BLAKE3_keyed(ns_secret_slot,
    /// "…/ns/v1" ‖ group_id ‖ slot ‖ rz_peer)[..20])`. The per-rendezvous binding
    /// (`rz_peer`) gives each rendezvous a string unique to itself, so two
    /// colluding rendezvous cannot join their logs on an identical namespace.
    fn derive_namespace(&self, rz_peer: &[u8], slot: u64) -> Option<String> {
        let secret = self.routing_secrets.get(&slot)?;
        // Canonical, length-prefixed preimage (matches the topic + tag hashers) so the
        // group_id ‖ rz_peer boundary is unambiguous.
        let mut e = Encoder::new();
        e.put_str("catcoms/rendezvous/ns/v1").expect("label fits");
        e.put_bytes(&self.group.group_id()).expect("group id fits");
        e.put_u64(slot);
        e.put_bytes(rz_peer).expect("rz peer fits");
        let hash = blake3::keyed_hash(secret, &e.finish());
        let hex = hash.to_hex();
        // 20 bytes / 160 bits of the keyed hash is ample collision resistance and
        // keeps the namespace short (49 ASCII bytes, well under the 255-byte cap).
        Some(format!("catcoms1-{}", &hex.as_str()[..40]))
    }

    /// The rendezvous namespace(s) to register under / discover across for this
    /// group at this rendezvous: the current label first, then every still-retained
    /// grandfathered label (`{L-1, L-2}`), so a member up to two removals behind is
    /// still found during the transition. A **removed** member cannot compute the
    /// current namespace (it never snapshots the post-removal secret). Each is
    /// ≤ 255 bytes, so `Namespace::new` accepts it.
    pub fn rendezvous_namespaces(&self, rz_peer: &[u8]) -> Vec<String> {
        let lowest = self.routing_label.saturating_sub(2);
        let mut out = Vec::new();
        for slot in (lowest..=self.routing_label).rev() {
            if let Some(ns) = self.derive_namespace(rz_peer, slot) {
                if !out.contains(&ns) {
                    out.push(ns);
                }
            }
        }
        out
    }

    /// Map a discovered namespace string back to its routing label, if it is one this
    /// node currently registers/discovers under (current or grandfathered). Used to
    /// know which `ns_secret_slot` a discovered record's tag must verify under.
    fn namespace_slot(&self, rz_peer: &[u8], namespace: &str) -> Option<u64> {
        let lowest = self.routing_label.saturating_sub(2);
        (lowest..=self.routing_label)
            .rev()
            .find(|&slot| self.derive_namespace(rz_peer, slot).as_deref() == Some(namespace))
    }

    /// The member-verifiable registration tag THIS node attaches when registering its
    /// own record `(peer_id, seq)` under routing label `slot` at rendezvous `rz_peer`.
    /// `None` if that label's routing secret is no longer retained. See
    /// [`routing_membership_tag`].
    pub fn membership_tag(
        &self,
        rz_peer: &[u8],
        slot: u64,
        peer_id: &[u8],
        seq: u64,
    ) -> Option<[u8; 16]> {
        let secret = self.routing_secrets.get(&slot)?;
        Some(routing_membership_tag(
            secret,
            &self.group.group_id(),
            rz_peer,
            slot,
            peer_id,
            seq,
        ))
    }

    /// Verify a discovered record's registration tag **before dialing** it: resolve
    /// the routing label from the namespace it was discovered under (current or
    /// grandfathered), recompute the tag under that label's `ns_secret`, and
    /// constant-time compare. Fails closed for a namespace this node does not
    /// recognise (an attacker's, or a label outside the grandfather window) and for a
    /// tag bound to a different `peer_id`/`seq`. This is the input to the
    /// `DiscoveryPolicy`'s `tag_verified` flag.
    pub fn verify_membership_tag(
        &self,
        rz_peer: &[u8],
        namespace: &str,
        peer_id: &[u8],
        seq: u64,
        tag: &[u8; 16],
    ) -> bool {
        let Some(slot) = self.namespace_slot(rz_peer, namespace) else {
            return false;
        };
        match self.membership_tag(rz_peer, slot, peer_id, seq) {
            Some(expected) => ct_eq_16(&expected, tag),
            None => false,
        }
    }

    // --- member peer exchange (PEX, 6e-3d-7) ---------------------------------

    /// Publish (or refresh) THIS node's own signed peer record with its dialable
    /// `addresses` and a monotonic `seq`, so it can be shared with other members via
    /// PEX. Signed by this device key; stored among the known records. Over-long
    /// addresses are dropped and the list is truncated to `MAX_PEX_ADDRESSES`; `seq`
    /// must be below `u64::MAX` (the top value is reserved so a node can always
    /// publish a fresher record later).
    pub fn publish_self_record(
        &mut self,
        mut addresses: Vec<String>,
        seq: u64,
    ) -> Result<(), SyncError> {
        if seq == u64::MAX {
            return Err(SyncError::Malformed);
        }
        addresses.retain(|a| a.len() <= MAX_PEX_ADDR_LEN);
        addresses.truncate(MAX_PEX_ADDRESSES);
        let device_pubkey = self.device.public_key_bytes();
        let peer_id = *self.transport.local_peer().as_bytes();
        let payload = peer_record_signing_payload(&device_pubkey, &peer_id, &addresses, seq);
        let signature = self.device.sign(&payload)?;
        let desc = PeerDescriptor {
            device_pubkey,
            peer_id,
            addresses,
            seq,
            signature,
        };
        self.peer_records.insert(self.device.device_id(), desc);
        Ok(())
    }

    /// This node's own published peer record, if any (to share / for tests).
    pub fn self_record(&self) -> Option<&PeerDescriptor> {
        self.peer_records.get(&self.device.device_id())
    }

    /// A known member's peer record, if learned (for the discovery layer / tests).
    pub fn peer_record(&self, device: &DeviceId) -> Option<&PeerDescriptor> {
        self.peer_records.get(device)
    }

    /// Every known member peer record (the discovery layer turns these into
    /// PEX-sourced dial candidates).
    pub fn known_peer_records(&self) -> Vec<PeerDescriptor> {
        self.peer_records.values().cloned().collect()
    }

    /// Verify and store a peer record: its **self-signature** must be valid **and**
    /// its signer must be a current group member. Returns `true` only when it adds a
    /// *newly-known* member (a refresh of an existing device, a stale-`seq` record, an
    /// invalid signature, or a non-member returns `false`). Used by PEX ingestion and
    /// by the net layer to feed discovered records — a record never bypasses these two
    /// checks, so a PEX responder cannot fabricate a peer's address or inject a Sybil.
    pub fn ingest_peer_record(&mut self, desc: PeerDescriptor) -> bool {
        if desc.seq == u64::MAX
            || desc.addresses.len() > MAX_PEX_ADDRESSES
            || desc.addresses.iter().any(|a| a.len() > MAX_PEX_ADDR_LEN)
        {
            return false;
        }
        // Cheap roster check BEFORE the Ed25519 self-signature verify, so a record
        // naming a non-member is dropped without spending a signature verification.
        let device = DeviceId::from_public_key_bytes(&desc.device_pubkey);
        if !self.group.contains_device(&device) {
            tracing::trace!("dropping peer record from a non-member");
            return false;
        }
        if !desc.verify_self() {
            tracing::trace!("dropping peer record with an invalid self-signature");
            return false;
        }
        let is_new = !self.peer_records.contains_key(&device);
        let store = match self.peer_records.get(&device) {
            Some(existing) => desc.seq > existing.seq, // keep the freshest by signed seq
            None => true,
        };
        if store {
            self.peer_records.insert(device, desc);
            self.bound_peer_records();
        }
        is_new
    }

    /// Bound the known-records map to `MAX_PEX_ENTRIES`, never evicting our own record.
    fn bound_peer_records(&mut self) {
        let own = self.device.device_id();
        while self.peer_records.len() > MAX_PEX_ENTRIES {
            let victim = self.peer_records.keys().find(|d| **d != own).cloned();
            match victim {
                Some(v) => {
                    self.peer_records.remove(&v);
                }
                None => break,
            }
        }
    }

    /// Record that we served a PEX response to requester `device` at `now`, for the
    /// rate limit; bound the map by evicting the stalest entry.
    fn note_pex_served(&mut self, device: DeviceId, now: u64) {
        self.pex_served_at.insert(device, now);
        while self.pex_served_at.len() > self.config.max_known_peers {
            let victim = self
                .pex_served_at
                .iter()
                .min_by_key(|(_, &t)| t)
                .map(|(p, _)| *p);
            match victim {
                Some(v) => {
                    self.pex_served_at.remove(&v);
                }
                None => break,
            }
        }
    }

    /// Ask `peer` (a member) for the signed peer records it knows (PEX). The response
    /// must be signed by a current member and bound to this request (anti-replay, like
    /// commit catch-up); each entry is then independently verified + membership-checked
    /// by [`Self::ingest_peer_record`]. Returns the number of *newly-known* members
    /// learned. A non-member / unsigned / replayed response yields `0`.
    pub async fn request_pex(&mut self, peer: PeerId) -> Result<usize, SyncError> {
        let (req, req_auth) = self.build_authed_request(KIND_PEX, &[])?;
        let resp = self
            .transport
            .request(peer, ProtocolId(RR_PROTOCOL), Bytes::from(req))
            .await?;
        if resp.is_empty() {
            return Ok(0);
        }
        if resp.len() > MAX_PEX_RESPONSE {
            tracing::warn!(bytes = resp.len(), "oversized PEX response dropped");
            return Err(SyncError::Malformed);
        }
        // The outer (responder_pubkey ‖ sig ‖ bundle) framing is shared with commit
        // catch-up; the bundle is PEX-specific and the transcript uses the PEX domain.
        let (responder_pubkey, signature, bundle) = decode_signed_commit_resp(&resp)?;
        let responder = DeviceId::from_public_key_bytes(&responder_pubkey);
        if !self.group.contains_device(&responder) {
            tracing::warn!(?peer, "PEX response from a non-member; rejected");
            return Ok(0);
        }
        let my_pubkey = self.device.public_key_bytes();
        let transcript = pex_resp_transcript(
            &self.group.group_id(),
            &my_pubkey,
            req_auth.ts,
            &req_auth.nonce,
            req_auth.epoch,
            &bundle,
        );
        if !verify_with_public_bytes(&responder_pubkey, &transcript, &signature) {
            tracing::warn!(?peer, "PEX response signature invalid; rejected");
            return Ok(0);
        }
        let records = decode_pex_bundle(&bundle)?;
        let mut learned = 0;
        for r in records {
            if self.ingest_peer_record(r) {
                learned += 1;
            }
        }
        tracing::debug!(learned, "applied PEX response");
        Ok(learned)
    }

    /// Serve a PEX request: only to a proven current member (the membership-authed
    /// gate), rate-limited per requester, returning a responder-signed bundle of up to
    /// `MAX_PEX_ENTRIES` known member records bound to this request.
    fn serve_pex(&mut self, _from: PeerId, data: &[u8]) -> Option<Vec<u8>> {
        let (_inner, req_pubkey, req_auth) = self.authenticate_request(KIND_PEX, data)?;
        let requester = DeviceId::from_public_key_bytes(&req_pubkey);
        let now = self.clock.now_ms();
        if let Some(&last) = self.pex_served_at.get(&requester) {
            if now.saturating_sub(last) < MIN_PEX_INTERVAL_MS {
                tracing::trace!("PEX rate-limited; serving empty");
                return Some(Vec::new());
            }
        }
        self.note_pex_served(requester, now);
        // Serve only records whose signer is STILL a current member (don't relay a
        // removed member's stale address even before its record is evicted).
        let records: Vec<PeerDescriptor> = self
            .peer_records
            .iter()
            .filter(|(device, _)| self.group.contains_device(device))
            .map(|(_, rec)| rec.clone())
            .take(MAX_PEX_ENTRIES)
            .collect();
        if records.is_empty() {
            return Some(Vec::new());
        }
        let bundle = encode_pex_bundle(&records);
        let transcript = pex_resp_transcript(
            &self.group.group_id(),
            &req_pubkey,
            req_auth.ts,
            &req_auth.nonce,
            req_auth.epoch,
            &bundle,
        );
        let signature = self.device.sign(&transcript).ok()?;
        tracing::debug!(count = records.len(), "serving PEX");
        Some(encode_signed_commit_resp(
            &self.device.public_key_bytes(),
            &signature,
            &bundle,
        ))
    }

    /// Seal this member's routing state (`L` + the retained `ns_secret_L` history)
    /// for a peer joining at the **current** epoch, under the shared routing-transfer
    /// key both will derive. Returned empty (no transfer) if the key or seal fails —
    /// the joiner then keeps its local `L = 0` (correct only for the founder).
    /// Call at the post-merge epoch the joiner's Welcome lands them on.
    fn seal_routing_state(&mut self) -> Vec<u8> {
        let key = match self.group.routing_transfer_key(&self.device) {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(error = %e, "no routing-transfer key; sending no transfer");
                return Vec::new();
            }
        };
        let secrets: Vec<(u64, [u8; 32])> = self
            .routing_secrets
            .iter()
            .map(|(slot, s)| (*slot, **s))
            .collect();
        let plaintext = Zeroizing::new(encode_join_transfer(
            self.routing_label,
            &secrets,
            &self.file_wrap_key,
        ));
        match seal(&key, &plaintext, &mut self.rng) {
            Ok(blob) => encode_sealed(&blob),
            Err(e) => {
                tracing::warn!(error = %e, "could not seal routing transfer");
                Vec::new()
            }
        }
    }

    /// Adopt a routing state transferred at join (replacing the locally-initialised
    /// `L = 0`), so this joiner derives the same topics and namespaces as the group.
    /// A no-op for an empty transfer (keeps the local baseline).
    fn adopt_routing_state(&mut self, routing: RoutingState) {
        // Adopt the transferred file-wrap key first (Phase 9h) — independent of the routing
        // secrets, so the joiner shares the group's file key even at the L=0 baseline.
        if let Some(k) = routing.file_wrap_key {
            self.file_wrap_key = Zeroizing::new(k);
        }
        if routing.secrets.is_empty() {
            return;
        }
        self.routing_label = routing.label;
        self.routing_secrets = routing.secrets.into_iter().collect();
        // Clamp to the live {L-2, L-1, L} window: a transfer may carry up to
        // MAX_ROUTING_SECRETS entries, but only the window is a valid live invariant
        // (don't keep out-of-window key material a malicious inviter padded in).
        let cutoff = self.routing_label.saturating_sub(2);
        self.routing_secrets
            .retain(|l, _| *l >= cutoff && *l <= self.routing_label);
        self.recompute_topics();
        tracing::debug!(
            label = self.routing_label,
            secrets = self.routing_secrets.len(),
            "adopted transferred routing state"
        );
    }

    /// Append a commit to the retained log (so this node can serve catch-up),
    /// bounded by `max_commit_log`.
    fn record_commit(&mut self, record: CommitRecord) {
        self.commit_log.push_back(record);
        while self.commit_log.len() > self.config.max_commit_log {
            self.commit_log.pop_front();
        }
    }

    /// Whether `record` was produced by an **authorized committer**: a current
    /// member (found by roster lookup, since a device id is a one-way hash of the
    /// key) whose per-commit signature verifies, of leaf rank no greater than
    /// `max_committer_rank` above the designated committer. openmls still
    /// independently authenticates the inner commit at `process_incoming`; this gate
    /// authorizes *who* may advance the epoch (and, for 6d-2, lets the fork-winner
    /// — who is not necessarily the *lowest*-index committer — be accepted).
    fn authorize_committer(&self, record: &CommitRecord) -> bool {
        let committer = DeviceId::from_bytes(record.committer_device);
        // 1. The committer must be a current member; look up its raw leaf key.
        let Some(pubkey) = self.group.member_signature_key(&committer) else {
            tracing::warn!(
                commit_epoch = record.commit_epoch,
                "commit from a non-member committer; rejected"
            );
            return false;
        };
        // 2. The per-commit signature must verify under that key.
        if !verify_with_public_bytes(&pubkey, &record.auth_transcript(), &record.committer_sig) {
            tracing::warn!(
                commit_epoch = record.commit_epoch,
                "commit signature invalid; rejected"
            );
            return false;
        }
        // 3. Authority bound: the committer's leaf rank (distance above the
        // designated committer) must be within the allowed window.
        match (
            self.group.member_leaf_index(&committer),
            self.group.designated_committer_index(),
        ) {
            (Some(idx), Some(base))
                if idx.saturating_sub(base) <= self.config.max_committer_rank =>
            {
                true
            }
            _ => {
                tracing::warn!(
                    commit_epoch = record.commit_epoch,
                    "commit from a committer outside the allowed rank; rejected"
                );
                false
            }
        }
    }

    /// Stage and broadcast a membership Remove as a fork-resolvable commit, then
    /// resolve it after the contest window. Requires `max_committer_rank >= 1`
    /// (concurrent committers enabled); with the default strict single-committer
    /// config, remove via the local MLS path instead. Errors if a commit is
    /// already staged here (one pending commit at a time).
    pub async fn remove(&mut self, target: &DeviceId) -> Result<(), SyncError> {
        if self.pending.is_some() {
            tracing::warn!("a commit is already staged here; remove deferred");
            return Err(SyncError::JoinRejected);
        }
        let staged = self.group.stage_remove(&self.device, target)?;
        let record = self.sign_staged_record(&staged);
        let mine = MyStaged {
            commit_id: record.commit_id(),
            join: None,
            removed: true,
        };
        self.pending = Some(PendingResolve {
            epoch: staged.commit_epoch,
            best: record.clone(),
            deadline_ms: self.clock.now_ms() + self.config.stage_decision_window_ms,
            mine: Some(mine),
        });
        let mut framed = vec![CTRL_COMMIT];
        framed.extend_from_slice(&record.encode());
        self.outbox.push((self.control_topic.clone(), framed));
        self.drain_outbox().await;
        Ok(())
    }

    /// Build the signed [`CommitRecord`] for one of our own staged commits.
    fn sign_staged_record(&self, staged: &catcoms_mls::StagedOutcome) -> CommitRecord {
        let committer_device = *self.device.device_id().as_bytes();
        let auth = commit_auth_transcript(
            &self.group.group_id(),
            staged.commit_epoch,
            &staged.base_authenticator,
            &committer_device,
            &staged.commit,
        );
        let committer_sig = self.device.sign(&auth).expect("sign own commit");
        CommitRecord {
            group_id: self.group.group_id(),
            commit_epoch: staged.commit_epoch,
            committer_device,
            mls_commit: staged.commit.clone(),
            base_authenticator: staged.base_authenticator,
            committer_sig,
        }
    }

    /// Fold an inbound commit into the fork-resolution contest for the current
    /// epoch: authorize it, reject a different-base (too-deep) candidate, else
    /// track the lowest `commit_id` seen. Nothing is applied until the window
    /// closes (see [`ChannelSync::finalize_contest`]).
    fn contest_commit(&mut self, record: CommitRecord) {
        // Drop any contest left over from an epoch we have since advanced past
        // (e.g. via commit catch-up) so we never fold a candidate into stale state.
        self.discard_stale_contest();
        if !self.authorize_committer(&record) {
            return;
        }
        let our_base = self.group.epoch_authenticator_id();
        if record.base_authenticator != our_base {
            tracing::warn!(
                commit_epoch = record.commit_epoch,
                "candidate built on a different base (fork too deep); refusing to tie-break"
            );
            self.stats.forks_too_deep += 1;
            return;
        }
        let current = self.group.epoch();
        match &mut self.pending {
            Some(p) if p.epoch == current => {
                // Lowest commit_id wins; ties (astronomically unlikely BLAKE3
                // collisions) break on the full commit bytes so the order is still
                // deterministic across nodes (I6).
                if (record.commit_id(), &record.mls_commit)
                    < (p.best.commit_id(), &p.best.mls_commit)
                {
                    tracing::debug!("lower-id competitor adopted as provisional fork winner");
                    p.best = record;
                }
            }
            _ => {
                // First candidate this epoch (we are a pure applier): open a contest.
                let deadline = self.clock.now_ms() + self.config.stage_decision_window_ms;
                self.pending = Some(PendingResolve {
                    epoch: current,
                    best: record,
                    deadline_ms: deadline,
                    mine: None,
                });
            }
        }
    }

    /// If a fork-resolution contest's window has closed, adopt the lowest-`commit_id`
    /// winner: the winning committer merges its staged commit; everyone else (and a
    /// losing committer, after aborting its own) applies the winner. Returns whether
    /// a contest was resolved this call.
    fn resolve_pending_if_expired(&mut self) -> bool {
        let (epoch, deadline) = match &self.pending {
            Some(p) => (p.epoch, p.deadline_ms),
            None => return false,
        };
        if epoch != self.group.epoch() {
            // A stale contest from an epoch we have since left: just drop it.
            self.pending = None;
            return false;
        }
        if self.clock.now_ms() < deadline {
            return false; // window still open
        }
        let p = self.pending.take().expect("checked some");
        let winner_id = p.best.commit_id();
        let we_won = p.mine.as_ref().map(|m| m.commit_id) == Some(winner_id);
        let i_removed = p.mine.as_ref().map(|m| m.removed).unwrap_or(false);
        self.snapshot_epoch_keys();
        let advanced = match &p.mine {
            Some(_) if we_won => match self.group.merge_staged_self(&self.device) {
                // We won: merge our own staged commit.
                Ok(()) => {
                    if i_removed {
                        self.rotate_routing_secret();
                    }
                    true
                }
                Err(e) => {
                    tracing::error!(error = %e, "merge of winning staged commit failed");
                    false
                }
            },
            Some(_) => {
                // We lost: roll our staged commit back, then apply the winner.
                self.stats.forks_lost += 1;
                if let Err(e) = self.group.abort_staged(&self.device) {
                    tracing::error!(error = %e, "abort of losing staged commit failed");
                }
                match self
                    .group
                    .process_incoming(&self.device, &p.best.mls_commit)
                {
                    Ok(inc) => {
                        self.note_commit_applied(&inc);
                        true
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "applying the fork winner failed");
                        false
                    }
                }
            }
            None => match self
                .group
                .process_incoming(&self.device, &p.best.mls_commit)
            {
                // Pure applier: apply the winner.
                Ok(inc) => {
                    self.note_commit_applied(&inc);
                    true
                }
                Err(e) => {
                    tracing::error!(error = %e, "applying the fork winner failed");
                    false
                }
            },
        };
        // Deliver the provisional-Welcome outcome to any joiner we admitted: the
        // signed Welcome on a winning merge, an empty (rejection) push otherwise —
        // so a losing committer never strands the joiner on a dead commit.
        if let Some(join) = p.mine.and_then(|m| m.join) {
            let payload = if we_won && advanced {
                let _ = self.ledger.consume(join.nonce);
                // We just merged the staged Add, so we are at the exact epoch the
                // joiner's Welcome lands them on — seal the routing state and sign
                // the transcript binding it (the joiner verifies both together).
                let sealed_routing = self.seal_routing_state();
                let transcript = join_transcript(
                    &self.group.group_id(),
                    &join.nonce,
                    &join.welcome,
                    &sealed_routing,
                );
                match self.device.sign(&transcript) {
                    Ok(welcome_sig) => {
                        encode_welcome_push(&join.welcome, &welcome_sig, &sealed_routing)
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "could not sign Welcome push");
                        Vec::new()
                    }
                }
            } else {
                Vec::new() // empty => rejected; the joiner retries
            };
            self.welcome_outbox.push((join.joiner, payload));
        }
        if advanced {
            self.evict_past_keys();
            self.stats.commits_applied += 1;
            self.stats.forks_resolved += 1;
            tracing::info!(
                epoch = self.group.epoch(),
                we_won,
                "resolved membership fork"
            );
            self.record_commit(p.best);
            self.drain_pending_commits();
        } else {
            // I2: a storage/merge failure must not wedge the node — heal via the
            // existing commit-catch-up recovery path on the next tick.
            tracing::warn!("fork resolution did not advance the epoch; scheduling recovery");
            let here = self.group.epoch();
            self.enqueue_commit_catchup(here);
        }
        true
    }

    /// Apply a commit that is exactly the next one (`commit_epoch == current`),
    /// capturing the soon-to-be-superseded epoch's keys first. Returns whether it
    /// applied.
    fn apply_commit_in_order(&mut self, record: &CommitRecord) -> bool {
        debug_assert_eq!(record.commit_epoch, self.group.epoch());
        if !self.authorize_committer(record) {
            return false;
        }
        self.snapshot_epoch_keys();
        match self
            .group
            .process_incoming(&self.device, &record.mls_commit)
        {
            Ok(inc) => {
                self.evict_past_keys();
                self.note_commit_applied(&inc);
                self.record_commit(record.clone());
                self.stats.commits_applied += 1;
                tracing::info!(
                    epoch = self.group.epoch(),
                    commit_epoch = record.commit_epoch,
                    "applied membership commit"
                );
                true
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    commit_epoch = record.commit_epoch,
                    "failed to apply membership commit"
                );
                false
            }
        }
    }

    /// Buffer a commit that arrived ahead of our epoch and chase the gap.
    fn buffer_future_commit(&mut self, record: CommitRecord) {
        let current = self.group.epoch();
        let gap = record.commit_epoch.saturating_sub(current);
        if gap > self.config.max_commit_gap {
            tracing::warn!(
                commit_epoch = record.commit_epoch,
                current,
                "membership commit too far ahead; dropping (DoS bound)"
            );
            return;
        }
        tracing::debug!(
            commit_epoch = record.commit_epoch,
            current,
            gap,
            "buffering out-of-order membership commit; chasing the gap"
        );
        self.pending_commits
            .entry(record.commit_epoch)
            .or_insert(record);
        // Bound the buffer: if over cap, drop the furthest-future entries (the
        // nearest ones are the gap-fillers we actually need first).
        while self.pending_commits.len() > self.config.max_pending_commits {
            if let Some((&k, _)) = self.pending_commits.iter().next_back() {
                self.pending_commits.remove(&k);
            } else {
                break;
            }
        }
        self.stats.commits_buffered += 1;
        self.enqueue_commit_catchup(current);
    }

    /// Apply any buffered commits that now fill the immediate next slot, in order.
    fn drain_pending_commits(&mut self) {
        // Discard any buffered commit we have since passed (already applied).
        let current = self.group.epoch();
        self.pending_commits.retain(|&e, _| e >= current);
        loop {
            let current = self.group.epoch();
            let Some(record) = self.pending_commits.remove(&current) else {
                break;
            };
            if !self.apply_commit_in_order(&record) {
                break;
            }
        }
        // If a buffered commit advanced us past a fork contest we were holding, that
        // contest is now stale — drop it (and roll back our own staged commit) so we
        // never resolve or extend it against a superseded epoch (I3).
        self.discard_stale_contest();
    }

    /// Drop a fork contest whose epoch we have already advanced past (e.g. via
    /// commit catch-up / buffered-commit replay), aborting any local staged commit
    /// it tracked. Keeps `pending` consistent with the actual epoch on every path,
    /// not only inside `resolve_pending_if_expired`.
    fn discard_stale_contest(&mut self) {
        let stale = matches!(&self.pending, Some(p) if p.epoch < self.group.epoch());
        if !stale {
            return;
        }
        let had_staged = self.pending.as_ref().is_some_and(|p| p.mine.is_some());
        let contest_epoch = self.pending.as_ref().map(|p| p.epoch).unwrap_or(0);
        self.pending = None;
        if had_staged {
            // Our staged openmls commit was built on the now-superseded epoch.
            let _ = self.group.abort_staged(&self.device);
        }
        tracing::warn!(
            contest_epoch,
            current = self.group.epoch(),
            "discarded a stale fork contest after the epoch advanced via another path"
        );
    }

    /// Request that the group's **designated committer** remove `target` — the
    /// single-serializer model (6d-2b): any member can ask, but only the one
    /// committer executes, so no concurrent commits (and no fork) ever arise. The
    /// signed request is broadcast on the control topic; the committer validates and
    /// performs a synchronous remove. Safe under the default config (no contest
    /// window needed). If *this* node is already the designated committer, the
    /// removal is performed directly.
    pub async fn request_remove(&mut self, target: &DeviceId) -> Result<(), SyncError> {
        // Removal is owner-only (THREAT-MODEL R1): only the designated committer (the server
        // owner) may remove, and it does so directly. A non-owner caller is unauthorized — we
        // return an error rather than broadcasting a request the committer would only reject.
        if !self.group.is_designated_committer(&self.device) {
            return Err(SyncError::Unauthorized);
        }
        self.commit_remove_now(target);
        self.drain_outbox().await;
        Ok(())
    }

    /// Handle an inbound remove request: only the designated committer acts on it,
    /// and only after authenticating the requester as a current member with a fresh
    /// signature. The committer then performs a synchronous remove and fans out the
    /// resulting commit.
    fn on_remove_request(&mut self, data: &[u8]) {
        if !self.group.is_designated_committer(&self.device) {
            return; // some other node is the serializer
        }
        let (target, pubkey, ts, signature) = match decode_remove_request(data) {
            Ok(r) => r,
            Err(_) => return,
        };
        // Removal is **owner-only**, enforced here at the protocol layer (THREAT-MODEL R1).
        // The owner is the designated committer and removes *directly* (see `request_remove`),
        // so it never sends a remove request to itself — any inbound request is therefore from
        // a non-owner and is rejected. The signature check below still matters: it prevents a
        // modified client from forging a request that merely *claims* the owner's key, since
        // only the owner's private key can produce a valid signature over the transcript.
        let requester = DeviceId::from_public_key_bytes(&pubkey);
        if self.group.designated_committer() != Some(requester) {
            tracing::warn!("remove request from a non-owner; ignored");
            self.stats.requests_rejected += 1;
            return;
        }
        if self.clock.now_ms().abs_diff(ts) > MAX_REQUEST_AGE_MS {
            self.stats.requests_rejected += 1;
            return;
        }
        let transcript = remove_req_transcript(&self.group.group_id(), &target, &pubkey, ts);
        if !verify_with_public_bytes(&pubkey, &transcript, &signature) {
            tracing::warn!("remove request signature invalid; ignored");
            self.stats.requests_rejected += 1;
            return;
        }
        let target_id = DeviceId::from_bytes(target);
        if !self.group.contains_device(&target_id) {
            return; // already gone / never a member
        }
        self.commit_remove_now(&target_id);
    }

    /// Perform a removal **as the designated committer**, synchronously (no contest):
    /// stage the Remove, capture the pre-advance keys, merge, and queue the signed
    /// commit for fan-out. Single-committer, so it cannot fork.
    fn commit_remove_now(&mut self, target: &DeviceId) {
        let staged = match self.group.stage_remove(&self.device, target) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "stage_remove failed; remove not performed");
                return;
            }
        };
        let record = self.sign_staged_record(&staged);
        self.snapshot_epoch_keys();
        if let Err(e) = self.group.merge_staged_self(&self.device) {
            tracing::error!(error = %e, "merge of remove commit failed");
            let _ = self.group.abort_staged(&self.device);
            return;
        }
        self.evict_past_keys();
        // Publish the removal commit on the PRE-rotation control topic — where the
        // members being removed-from are still subscribed — *then* rotate the label
        // (the removed member cannot export the post-removal routing secret).
        let publish_topic = self.control_topic.clone();
        self.rotate_routing_secret();
        self.record_commit(record.clone());
        self.stats.commits_applied += 1;
        let mut framed = vec![CTRL_COMMIT];
        framed.extend_from_slice(&record.encode());
        self.outbox.push((publish_topic, framed));
        tracing::info!(
            epoch = self.group.epoch(),
            "committer removed a member (single-serializer)"
        );
    }

    /// Apply an inbound membership commit from the control topic. A commit that is
    /// exactly the next one is applied immediately (then any buffered successors
    /// drain in order); one that is ahead of us is buffered and triggers
    /// commit-catch-up; an already-applied one is ignored.
    fn on_control(&mut self, from: PeerId, data: &[u8]) {
        // The control envelope is a tagged union (commit record / remove request / add request).
        let record = match data.split_first() {
            Some((&CTRL_COMMIT, rest)) => match CommitRecord::decode(rest) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "dropping malformed control message");
                    return;
                }
            },
            Some((&CTRL_REMOVE_REQUEST, rest)) => {
                self.on_remove_request(rest);
                return;
            }
            Some((&CTRL_ADD_REQUEST, rest)) => {
                // `from` is the requesting admin's peer — where the owner returns the result.
                self.on_add_request(from, rest);
                return;
            }
            _ => {
                tracing::warn!("dropping control message with unknown tag");
                return;
            }
        };
        if record.group_id != self.group.group_id() {
            return;
        }
        let current = self.group.epoch();
        match record.commit_epoch.cmp(&current) {
            Ordering::Less => {
                tracing::trace!(
                    commit_epoch = record.commit_epoch,
                    current,
                    "ignoring already-applied membership commit"
                );
            }
            Ordering::Equal => {
                if self.config.max_committer_rank == 0 {
                    // Single-committer fast path: no concurrent committers exist, so
                    // no fork is possible — apply immediately (6d-1 behavior).
                    if self.apply_commit_in_order(&record) {
                        self.drain_pending_commits();
                    }
                } else {
                    // Concurrent committers are allowed: run a fork-resolution
                    // contest (collect competing same-base commits, adopt the
                    // lowest commit_id when the window closes).
                    self.contest_commit(record);
                }
            }
            Ordering::Greater => self.buffer_future_commit(record),
        }
    }

    /// Request a catch-up bundle for a document from `peer` and apply it.
    /// Returns the number of newly-applied ops.
    pub async fn request_catchup(
        &mut self,
        peer: catcoms_rt::PeerId,
        doc_type: DocType,
        doc_id: u128,
    ) -> Result<usize, SyncError> {
        let (req, _auth) =
            self.build_authed_request(KIND_CATCHUP, &encode_catchup_req(doc_type, doc_id))?;
        tracing::debug!(?doc_type, doc_id, ?peer, "request doc catch-up");
        self.stats.doc_catchups_requested += 1;
        let resp = self
            .transport
            .request(peer, ProtocolId(RR_PROTOCOL), Bytes::from(req))
            .await?;
        if resp.is_empty() {
            return Ok(0); // peer had nothing for this document
        }
        if resp.len() > MAX_CATCHUP_RESPONSE {
            tracing::warn!(
                bytes = resp.len(),
                "oversized doc catch-up response dropped"
            );
            return Err(SyncError::Malformed);
        }
        let bundle = decode_bundle(&resp)?;

        let key = (doc_type, doc_id);
        let actor = self.device.device_id();
        self.docs
            .entry(key)
            .or_insert_with(|| EncryptedDoc::new(doc_type, doc_id, &actor));
        let doc = self.docs.get_mut(&key).expect("just inserted");
        let applied = doc.import_catchup(&bundle, &self.group, &self.device)?;
        tracing::debug!(applied, "applied doc catch-up");
        Ok(applied)
    }

    /// Catch a document up from the **best known peer** — a proven member first, falling
    /// back to any known peer (see [`Self::pick_catchup_peer`]). Returns `Ok(0)` if there
    /// is no peer to ask yet. Unlike [`Self::request_catchup`] (which targets a specific
    /// peer, e.g. the inviter), this works for any member with a populated peer pool — so
    /// either side can pull the backlog of a channel the other created.
    pub async fn request_catchup_best(
        &mut self,
        doc_type: DocType,
        doc_id: u128,
    ) -> Result<usize, SyncError> {
        match self.pick_catchup_peer() {
            Some(peer) => self.request_catchup(peer, doc_type, doc_id).await,
            None => Ok(0),
        }
    }

    /// Store a blob locally, returning its content address (8l).
    pub fn put_blob(&mut self, bytes: &[u8]) -> Result<Cid, SyncError> {
        Ok(self.blobs.put(bytes)?)
    }

    /// Fetch a locally-held blob by content address (`None` if not held).
    pub fn get_blob(&self, cid: &Cid) -> Option<Vec<u8>> {
        self.blobs.get(cid).ok().flatten()
    }

    /// Whether a blob is held locally.
    pub fn has_blob(&self, cid: &Cid) -> bool {
        self.blobs.has(cid)
    }

    /// Fetch a blob by content address from `peer`, verify it, and store it. Returns
    /// `Ok(true)` if fetched (or already held), `Ok(false)` if the peer did not have it.
    /// The response is members-only and signed (bound to this request); the served bytes
    /// are re-hashed against the requested address before storing, so a member cannot
    /// substitute different bytes under it.
    pub async fn request_blob(
        &mut self,
        peer: catcoms_rt::PeerId,
        cid: &Cid,
    ) -> Result<bool, SyncError> {
        if self.blobs.has(cid) {
            return Ok(true);
        }
        let (req, auth) =
            self.build_authed_request(KIND_BLOB_FETCH, &encode_blob_fetch_req(cid))?;
        let resp = self
            .transport
            .request(peer, ProtocolId(RR_PROTOCOL), Bytes::from(req))
            .await?;
        if resp.is_empty() {
            return Ok(false); // the peer did not have this blob
        }
        if resp.len() > MAX_BLOB_RESPONSE {
            tracing::warn!(bytes = resp.len(), "oversized blob response dropped");
            return Err(SyncError::Malformed);
        }
        let (responder_pubkey, signature, blob) = decode_signed_commit_resp(&resp)?;
        // The responder must be a current member, and the signature must bind this blob to
        // our exact request (key + ts + nonce + epoch).
        let responder = DeviceId::from_public_key_bytes(&responder_pubkey);
        if !self.group.contains_device(&responder) {
            return Err(SyncError::Malformed);
        }
        let transcript = blob_fetch_resp_transcript(
            &self.group.group_id(),
            &self.device.public_key_bytes(),
            auth.ts,
            &auth.nonce,
            auth.epoch,
            &blob,
        );
        if !verify_with_public_bytes(&responder_pubkey, &transcript, &signature) {
            tracing::warn!("blob response signature invalid; dropped");
            return Err(SyncError::Malformed);
        }
        // Verify the served bytes hash to the address we asked for *before* storing them.
        if Cid::of(&blob) != *cid {
            tracing::warn!("served blob content-address mismatch; dropped");
            return Err(SyncError::Malformed);
        }
        self.blobs.put(&blob)?;
        Ok(true)
    }

    /// Fetch a blob from the **best known peer** (a proven member, else any known peer).
    /// `Ok(false)` if there is no peer to ask, or the peer did not have it.
    pub async fn request_blob_best(&mut self, cid: &Cid) -> Result<bool, SyncError> {
        if self.blobs.has(cid) {
            return Ok(true);
        }
        match self.pick_catchup_peer() {
            Some(peer) => self.request_blob(peer, cid).await,
            None => Ok(false),
        }
    }

    /// Request missed membership commits from `peer` starting at `from_epoch` and
    /// replay them in order. Returns the number of commits applied. This is the
    /// recovery path for a member that missed a control-topic broadcast.
    pub async fn request_commit_catchup(
        &mut self,
        peer: catcoms_rt::PeerId,
        from_epoch: u64,
    ) -> Result<usize, SyncError> {
        self.do_commit_catchup(peer, from_epoch).await
    }

    async fn do_commit_catchup(
        &mut self,
        peer: PeerId,
        from_epoch: u64,
    ) -> Result<usize, SyncError> {
        let (req, req_auth) =
            self.build_authed_request(KIND_COMMIT_CATCHUP, &encode_commit_catchup_req(from_epoch))?;
        tracing::debug!(from_epoch, ?peer, "request commit catch-up");
        self.stats.commit_catchups_requested += 1;
        let resp = self
            .transport
            .request(peer, ProtocolId(RR_PROTOCOL), Bytes::from(req))
            .await?;
        if resp.is_empty() {
            return Ok(0);
        }
        if resp.len() > MAX_CATCHUP_RESPONSE {
            tracing::warn!(
                bytes = resp.len(),
                "oversized commit catch-up response dropped"
            );
            return Err(SyncError::Malformed);
        }
        // The response must be signed by a current member, bound to THIS request — so
        // an un-handshaked peer cannot feed us a trusted bundle or be promoted as a
        // catch-up source (6e-3d-5, the Sybil-C1 fix). Anti-replay binds the response
        // to the request timestamp **and** a per-request nonce + the requester epoch
        // (6e-3d-6, below), closing the same-millisecond `ts`-collision window — so a
        // captured member response cannot be replayed against a different request. An
        // invalid response fills no gap, so the drain marks it failed.
        let (responder_pubkey, signature, bundle) = decode_signed_commit_resp(&resp)?;
        let group_id = self.group.group_id();
        let responder = DeviceId::from_public_key_bytes(&responder_pubkey);
        let my_pubkey = self.device.public_key_bytes();
        // Reconstruct the response transcript with OUR remembered request metadata
        // (ts/nonce/epoch). A relay that tampered the nonce in transit makes the
        // responder sign a different transcript than we reconstruct → verify fails.
        let transcript = catchup_resp_transcript(
            &group_id,
            &my_pubkey,
            req_auth.ts,
            &req_auth.nonce,
            req_auth.epoch,
            &bundle,
        );
        if !self.group.contains_device(&responder) {
            // Not in the current roster (e.g. a since-removed member): demote it from
            // the trusted pool so it is no longer preferred, and reject the bundle.
            self.demote_member_peer(peer);
            tracing::warn!(
                ?peer,
                "commit catch-up response from a non-member; demoted + rejected"
            );
            return Ok(0);
        }
        if !verify_with_public_bytes(&responder_pubkey, &transcript, &signature) {
            tracing::warn!(
                ?peer,
                "commit catch-up response signature invalid; rejected"
            );
            return Ok(0);
        }
        // The responder proved current membership: trust the bundle and promote it to
        // the verified catch-up-source pool.
        self.promote_member_peer(peer);
        let records = decode_commit_bundle(&bundle)?;
        let mut applied = 0;
        for record in records {
            if record.group_id != group_id {
                continue;
            }
            let current = self.group.epoch();
            match record.commit_epoch.cmp(&current) {
                Ordering::Equal => {
                    if self.apply_commit_in_order(&record) {
                        applied += 1;
                    }
                }
                Ordering::Greater => self.buffer_future_commit(record),
                Ordering::Less => {}
            }
        }
        // Buffered successors that the fetch unblocked now drain in order.
        self.drain_pending_commits();
        if let Some((&lowest_gap, _)) = self.pending_commits.iter().next() {
            if lowest_gap > self.group.epoch() {
                tracing::warn!(
                    current = self.group.epoch(),
                    lowest_gap,
                    "commit catch-up left an unfillable gap (source's window evicted it); a full rejoin/snapshot is needed"
                );
            }
        }
        tracing::debug!(
            applied,
            epoch = self.group.epoch(),
            "applied commit catch-up"
        );
        Ok(applied)
    }

    /// Read a document's materialized state.
    pub fn doc(&self, doc_type: DocType, doc_id: u128) -> Option<&EncryptedDoc> {
        self.docs.get(&(doc_type, doc_id))
    }

    /// This member's transport peer id.
    pub fn local_peer(&self) -> catcoms_rt::PeerId {
        self.transport.local_peer()
    }

    /// The current time in epoch-millis from the injected clock (no ambient time). Used by
    /// the product layer to stamp content (e.g. message timestamps).
    pub fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Borrow the underlying transport, so the **discovery/dial layer** — which lives
    /// *above* `ChannelSync` (the net Actor never auto-dials; the dial decision and
    /// eclipse-resistance are a layer up) — can drive rendezvous register/discover and
    /// dial discovered peers. `ChannelSync` itself never dials.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Whether `device` is a current member of this group (for tests/diagnostics).
    pub fn contains_member(&self, device: &DeviceId) -> bool {
        self.group.contains_device(device)
    }

    /// The current member count (for tests/diagnostics).
    pub fn member_count(&self) -> usize {
        self.group.member_count()
    }

    /// The current roster — the device ids of all members (for the UI/product layer).
    pub fn member_ids(&self) -> Vec<DeviceId> {
        self.group.member_device_ids()
    }

    fn on_gossip(&mut self, data: &[u8]) {
        let sealed = match SealedOp::decode(data) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "dropping malformed gossip op");
                return;
            }
        };
        let key = (sealed.doc_type, sealed.doc_id);
        if !self.docs.contains_key(&key) {
            tracing::trace!(?key, "gossip for unopened document, ignoring");
            return;
        }
        let current = self.group.epoch();
        match sealed.epoch.cmp(&current) {
            Ordering::Equal => self.ingest_current(&sealed),
            Ordering::Less => self.ingest_past(&sealed),
            Ordering::Greater => self.ingest_future(&sealed),
        }
    }

    /// Ingest an op sealed under the current epoch (the common path).
    fn ingest_current(&mut self, sealed: &SealedOp) {
        let key = (sealed.doc_type, sealed.doc_id);
        let doc = self.docs.get_mut(&key).expect("checked open");
        match doc.ingest(sealed, &self.group, &self.device) {
            Ok(true) => {
                self.stats.ops_ingested += 1;
                tracing::trace!(?key, "ingested op");
            }
            Ok(false) => tracing::trace!(?key, "duplicate op ignored"),
            Err(e) => tracing::warn!(error = %e, "rejected inbound op"),
        }
    }

    /// Ingest an op sealed under a past epoch, using a retained key if the epoch
    /// is still within the window; otherwise fall back to a document catch-up.
    fn ingest_past(&mut self, sealed: &SealedOp) {
        let pk = (sealed.doc_type, sealed.doc_id, sealed.epoch);
        // Copy the key out (re-wrapped in Zeroizing so the local copy is also
        // scrubbed on drop) so the `past_keys` borrow ends before we touch
        // `docs`/`stats`/`catchup_queue`.
        if let Some(key) = self.past_keys.get(&pk).map(|k| Zeroizing::new(**k)) {
            let doc = self
                .docs
                .get_mut(&(sealed.doc_type, sealed.doc_id))
                .expect("checked open");
            match doc.ingest_with_key(sealed, sealed.epoch, &key) {
                Ok(true) => {
                    self.stats.ops_recovered_past_epoch += 1;
                    tracing::debug!(epoch = sealed.epoch, "recovered op across epoch boundary");
                }
                Ok(false) => tracing::trace!("duplicate past-epoch op ignored"),
                Err(e) => tracing::warn!(error = %e, "rejected past-epoch op"),
            }
        } else {
            self.stats.ops_dropped_old_epoch += 1;
            tracing::debug!(
                epoch = sealed.epoch,
                current = self.group.epoch(),
                "op sealed under an evicted past epoch; requesting doc catch-up"
            );
            self.enqueue_doc_catchup(sealed.doc_type, sealed.doc_id);
        }
    }

    /// Handle an op sealed under a future epoch we have not yet reached: we are
    /// behind on membership commits. Chase the commits (to advance) and the
    /// document (to recover this op once we have caught up) rather than dropping
    /// it silently.
    fn ingest_future(&mut self, sealed: &SealedOp) {
        self.stats.ops_dropped_future_epoch += 1;
        let current = self.group.epoch();
        tracing::debug!(
            epoch = sealed.epoch,
            current,
            "op sealed under a future epoch; chasing commits + doc catch-up"
        );
        self.enqueue_commit_catchup(current);
        self.enqueue_doc_catchup(sealed.doc_type, sealed.doc_id);
    }

    /// Route an inbound request by its kind byte. Returns the response bytes (an
    /// empty response uniformly signals "nothing / rejected"). `from` is the
    /// requesting peer (needed to push a staged join's Welcome back to it).
    fn handle_request(&mut self, from: PeerId, data: &[u8]) -> Vec<u8> {
        if data.len() > MAX_CONTROL_REQUEST {
            tracing::warn!(bytes = data.len(), "oversized control request dropped");
            return Vec::new();
        }
        match data.split_first() {
            Some((&KIND_CATCHUP, rest)) => self.serve_catchup(rest).unwrap_or_default(),
            Some((&KIND_JOIN, rest)) => self.serve_join(from, rest).unwrap_or_default(),
            Some((&KIND_COMMIT_CATCHUP, rest)) => {
                self.serve_commit_catchup(rest).unwrap_or_default()
            }
            Some((&KIND_PEX, rest)) => self.serve_pex(from, rest).unwrap_or_default(),
            Some((&KIND_BLOB_FETCH, rest)) => self.serve_blob_fetch(rest).unwrap_or_default(),
            Some((&KIND_ADMIT_RESULT, rest)) => {
                // Admin invites (Option C): the owner delivered a finalized admission; re-sign +
                // relay the Welcome to the joiner. Empty ack response.
                self.on_admit_result(rest);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Record that we served a blob to requester `device` at `now`, for the rate limit;
    /// bound the map by evicting the stalest entry (mirrors `note_pex_served`).
    fn note_blob_served(&mut self, device: DeviceId, now: u64) {
        self.blob_served_at.insert(device, now);
        while self.blob_served_at.len() > self.config.max_known_peers {
            let victim = self
                .blob_served_at
                .iter()
                .min_by_key(|(_, &t)| t)
                .map(|(p, _)| *p);
            match victim {
                Some(v) => {
                    self.blob_served_at.remove(&v);
                }
                None => break,
            }
        }
    }

    /// Serve a content-addressed blob — only to a proven current member, rate-limited per
    /// requester (blob is the strongest amplifier), signing the response bound to the
    /// requester's request. An empty response means not held (or rate-limited).
    fn serve_blob_fetch(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let (inner, req_pubkey, req_auth) = self.authenticate_request(KIND_BLOB_FETCH, data)?;
        let requester = DeviceId::from_public_key_bytes(&req_pubkey);
        let now = self.clock.now_ms();
        if let Some(&last) = self.blob_served_at.get(&requester) {
            if now.saturating_sub(last) < MIN_BLOB_INTERVAL_MS {
                tracing::trace!("blob fetch rate-limited; serving empty");
                return Some(Vec::new());
            }
        }
        let cid = decode_blob_fetch_req(&inner).ok()?;
        let blob = self.blobs.get(&cid).ok()??;
        if blob.len() > MAX_BLOB_RESPONSE {
            tracing::warn!(
                bytes = blob.len(),
                "held blob exceeds response budget; not served"
            );
            return None;
        }
        let transcript = blob_fetch_resp_transcript(
            &self.group.group_id(),
            &req_pubkey,
            req_auth.ts,
            &req_auth.nonce,
            req_auth.epoch,
            &blob,
        );
        let signature = self.device.sign(&transcript).ok()?;
        // Only the expensive serve path (a held blob, read + signed) counts toward the
        // rate limit; a miss is cheap and not throttled.
        self.note_blob_served(requester, now);
        tracing::debug!(bytes = blob.len(), "serving blob");
        Some(encode_signed_commit_resp(
            &self.device.public_key_bytes(),
            &signature,
            &blob,
        ))
    }

    /// Serve a document's history — but only to a requester that proved current
    /// group membership (the bundle, though sealed, still carries member-only
    /// framing/metadata, so it is members-only).
    fn serve_catchup(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let (inner, _pubkey, _auth) = self.authenticate_request(KIND_CATCHUP, data)?;
        let (doc_type, doc_id) = decode_catchup_req(&inner).ok()?;
        let doc = self.docs.get(&(doc_type, doc_id))?;
        match doc.export_catchup(&self.group, &self.device, &mut self.rng) {
            Ok(bundle) => {
                // Bound the response we generate (mirror the inbound cap): serve a
                // contiguous prefix that fits the byte budget; the requester pages
                // the rest with a follow-up catch-up.
                let (prefix, served) = size_capped_ops(&bundle, MAX_CONTROL_RESPONSE);
                if served < bundle.len() {
                    tracing::debug!(
                        served,
                        total = bundle.len(),
                        "doc catch-up truncated to response budget"
                    );
                }
                tracing::debug!(?doc_type, doc_id, ops = served, "serving doc catch-up");
                Some(prefix)
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to build catch-up bundle");
                None
            }
        }
    }

    /// Serve missed membership commits at or after `from_epoch` from the retained
    /// commit log, in epoch order — only to a proven current member (the records'
    /// framing reveals group id + member device ids, so this is members-only).
    fn serve_commit_catchup(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let (inner, req_pubkey, req_auth) = self.authenticate_request(KIND_COMMIT_CATCHUP, data)?;
        let from_epoch = decode_commit_catchup_req(&inner).ok()?;
        // Take a contiguous prefix from `from_epoch` that fits the byte budget.
        let mut records: Vec<CommitRecord> = Vec::new();
        let mut size = 4; // bundle count header
        for r in self
            .commit_log
            .iter()
            .filter(|r| r.commit_epoch >= from_epoch)
        {
            let entry = r.encode().len() + 4;
            if size + entry > MAX_CONTROL_RESPONSE {
                tracing::debug!("commit catch-up truncated to response budget");
                break;
            }
            size += entry;
            records.push(r.clone());
        }
        if records.is_empty() {
            return Some(Vec::new());
        }
        self.stats.commits_served += 1;
        tracing::debug!(from_epoch, count = records.len(), "serving commit catch-up");
        // Sign the bundle as a current member, bound to this requester + request, so
        // the requester can verify it came from a member before trusting it (and
        // promote us as a catch-up source). An attacker who is not in the roster
        // cannot produce a verifying signature.
        let bundle = encode_commit_bundle(&records);
        let transcript = catchup_resp_transcript(
            &self.group.group_id(),
            &req_pubkey,
            req_auth.ts,
            &req_auth.nonce,
            req_auth.epoch,
            &bundle,
        );
        let signature = self.device.sign(&transcript).ok()?;
        Some(encode_signed_commit_resp(
            &self.device.public_key_bytes(),
            &signature,
            &bundle,
        ))
    }

    /// Admit a joiner from a join request. Cheap, KeyPackage-independent checks run
    /// first (so junk requests never pay for KeyPackage validation); only the
    /// invite's named inviter admits (so the joiner can authenticate the Welcome
    /// against the invite's public key). With the default single-committer config
    /// the Add is merged and the signed Welcome returned synchronously (`JOIN_READY`).
    /// With concurrent committers enabled (`max_committer_rank >= 1`) the Add is
    /// **staged** into a fork-resolution contest and a `JOIN_PENDING` ack is
    /// returned; the signed Welcome is **pushed** to the joiner only once the staged
    /// commit wins and merges (so a losing committer never strands the joiner).
    fn serve_join(&mut self, from: PeerId, data: &[u8]) -> Option<Vec<u8>> {
        let (invite, kp_bytes) = decode_join_req(data).ok()?;

        // --- cheap checks first (no asymmetric crypto on the KeyPackage) ---
        if invite.group_id != self.group.group_id() {
            return None;
        }
        // Only the inviter named in the invite admits over the wire.
        if self.device.device_id() != invite.inviter_device_id {
            tracing::warn!("join request for an invite this device did not issue");
            return None;
        }
        if !invite.verify_self() {
            tracing::warn!("join request with an inauthentic invite");
            return None;
        }
        let now = self.clock.now_ms();
        if self.ledger.check(&invite, now).is_err() {
            return None; // expired / revoked / already used
        }

        // Where this node sits relative to the designated committer. At rank 0 this is exactly
        // the designated committer (the 6d-1 invariant).
        let my_rank = match (
            self.group.member_leaf_index(&self.device.device_id()),
            self.group.designated_committer_index(),
        ) {
            (Some(idx), Some(base)) => idx.saturating_sub(base),
            _ => return None,
        };

        // A non-committer (an Admin, in the single-committer model) cannot run the Add itself —
        // that would be a second committer (a fork). Instead it asks the owner to admit
        // (Option C): broadcast a signed Add-request, drive it to completion, and tell the joiner
        // to wait for the pushed Welcome. The OWNER re-checks the inviter's role authoritatively
        // (against its own live roles doc) before committing, so we don't self-gate here — our
        // local roles view may lag, and only the owner ever commits anyway (no fork).
        if my_rank > self.config.max_committer_rank {
            self.request_add(invite, kp_bytes, from, now);
            return Some(vec![JOIN_PENDING]);
        }

        if self.config.max_committer_rank == 0 {
            // --- synchronous single-committer path (6d-1 behavior) ---
            let (welcome, sealed_routing, signature) = self.admit_now(&invite, &kp_bytes, now)?;
            let mut resp = vec![JOIN_READY];
            resp.extend_from_slice(&encode_join_resp(&welcome, &signature, &sealed_routing));
            return Some(resp);
        }

        // --- staged two-phase path (fork-resolvable; provisional Welcome) ---
        if self.pending.is_some() {
            tracing::warn!("a commit is already staged here; rejecting concurrent join (retry)");
            return None; // joiner retries against the (now-known) committer
        }
        let key_package = self.device.parse_key_package(&kp_bytes).ok()?;
        self.group
            .validate_invite_binding(&key_package, &invite)
            .ok()?;
        let staged = self.group.stage_add(&self.device, key_package).ok()?;
        let welcome = staged.welcome.clone()?; // an Add always carries a Welcome
        let record = self.sign_staged_record(&staged);
        self.pending = Some(PendingResolve {
            epoch: staged.commit_epoch,
            best: record.clone(),
            deadline_ms: now + self.config.stage_decision_window_ms,
            mine: Some(MyStaged {
                commit_id: record.commit_id(),
                join: Some(StagedJoin {
                    joiner: from,
                    nonce: invite.invite_nonce,
                    welcome,
                }),
                removed: false,
            }),
        });
        let mut framed = vec![CTRL_COMMIT];
        framed.extend_from_slice(&record.encode());
        self.outbox.push((self.control_topic.clone(), framed));
        tracing::info!("staged an admission; awaiting the fork-resolution window");
        Some(vec![JOIN_PENDING])
    }

    /// Run the synchronous single-committer admission: produce + locally apply the MLS Add,
    /// broadcast the commit, seal the routing transfer, and sign the join transcript. Shared by
    /// `serve_join` (owner admitting its own invite) and `drain_add_request_queue` (owner
    /// admitting an admin-requested invite). Returns `(welcome, sealed_routing, owner_signature)`.
    fn admit_now(
        &mut self,
        invite: &InviteToken,
        kp_bytes: &[u8],
        now: u64,
    ) -> Option<(Vec<u8>, Vec<u8>, [u8; 64])> {
        let key_package = self.device.parse_key_package(kp_bytes).ok()?;
        let base_authenticator = self.group.epoch_authenticator_id();
        self.snapshot_epoch_keys();
        let outcome = self
            .group
            .add_member_via_invite(&self.device, key_package, invite, &mut self.ledger, now)
            .ok()?;
        self.evict_past_keys();
        let record =
            self.sign_add_record(outcome.commit_epoch, &outcome.commit, base_authenticator);
        self.record_commit(record.clone());
        let mut framed = vec![CTRL_COMMIT];
        framed.extend_from_slice(&record.encode());
        self.outbox.push((self.control_topic.clone(), framed));
        // Seal the routing transfer first so the join transcript binds it (the joiner verifies
        // the Welcome + routing together).
        let sealed_routing = self.seal_routing_state();
        let transcript = join_transcript(
            &invite.group_id,
            &invite.invite_nonce,
            &outcome.welcome,
            &sealed_routing,
        );
        let signature = self.device.sign(&transcript).ok()?;
        tracing::info!(epoch = self.group.epoch(), "admitted a member via invite");
        Some((outcome.welcome, sealed_routing, signature))
    }

    /// Admin side (Option C): broadcast a signed Add-request asking the owner to admit `invite`'s
    /// joiner, and remember it so we drive it to completion (re-broadcast until the owner returns
    /// the result, then re-sign + push the Welcome to the joiner).
    fn request_add(&mut self, invite: InviteToken, kp_bytes: Vec<u8>, joiner: PeerId, now: u64) {
        let kp_hash = Cid::of(&kp_bytes);
        let pubkey = self.device.public_key_bytes();
        let transcript = add_req_transcript(
            &self.group.group_id(),
            &invite.invite_nonce,
            kp_hash.as_bytes(),
            &pubkey,
            now,
        );
        let Ok(sig) = self.device.sign(&transcript) else {
            return;
        };
        let mut framed = vec![CTRL_ADD_REQUEST];
        framed.extend_from_slice(&encode_add_request(
            &invite.encode(),
            &kp_bytes,
            &pubkey,
            now,
            &sig,
        ));
        self.outbox.push((self.control_topic.clone(), framed));
        // Bound how long we drive this request (don't trust a far-future invite expiry), and cap
        // the map (evict the soonest-expiring) so a busy admin can't grow it without bound.
        let expires_at_ms = invite
            .expires_at_ms
            .min(now.saturating_add(MAX_ADD_REQUEST_LIFETIME_MS));
        while self.outgoing_add_requests.len() >= MAX_ADD_REQUESTS {
            let Some(victim) = self
                .outgoing_add_requests
                .iter()
                .min_by_key(|(_, o)| o.expires_at_ms)
                .map(|(k, _)| *k)
            else {
                break;
            };
            self.outgoing_add_requests.remove(&victim);
        }
        self.outgoing_add_requests.insert(
            invite.invite_nonce,
            OutgoingAdd {
                expires_at_ms,
                invite,
                kp_bytes,
                joiner,
                next_retry_ms: now + ADD_REQ_RETRY_MS,
            },
        );
    }

    /// Admin side: re-broadcast any pending Add-request whose retry interval elapsed (and drop
    /// expired ones). Driven off `run_once` events — notably the owner's reconnect — so an
    /// offline owner is caught up when it returns.
    fn drive_outgoing_add_requests(&mut self) {
        let now = self.clock.now_ms();
        // (invite_encoded, kp_bytes, invite_nonce, group_id) for each request to re-broadcast.
        let mut to_send: Vec<Rebroadcast> = Vec::new();
        self.outgoing_add_requests.retain(|nonce, out| {
            if now > out.expires_at_ms {
                return false; // expired; give up driving it
            }
            if now >= out.next_retry_ms {
                out.next_retry_ms = now + ADD_REQ_RETRY_MS;
                to_send.push((
                    out.invite.encode(),
                    out.kp_bytes.clone(),
                    *nonce,
                    out.invite.group_id.clone(),
                ));
            }
            true
        });
        let pubkey = self.device.public_key_bytes();
        for (invite_enc, kp_bytes, nonce, group_id) in to_send {
            let kp_hash = Cid::of(&kp_bytes);
            let transcript =
                add_req_transcript(&group_id, &nonce, kp_hash.as_bytes(), &pubkey, now);
            if let Ok(sig) = self.device.sign(&transcript) {
                let mut framed = vec![CTRL_ADD_REQUEST];
                framed.extend_from_slice(&encode_add_request(
                    &invite_enc,
                    &kp_bytes,
                    &pubkey,
                    now,
                    &sig,
                ));
                self.outbox.push((self.control_topic.clone(), framed));
            }
        }
    }

    /// Owner side (Option C): an authorized admin asked us to admit a joiner via an invite.
    /// Re-check everything against the *live* group + roles doc, then queue the admission (or, if
    /// we already admitted this nonce, re-deliver the cached result). Only the owner acts.
    // `from_admin` is the gossip *source* (the broadcasting admin), used to route the admit
    // result back. This is the authenticated publisher only because the gossipsub layer is
    // configured `MessageAuthenticity::Signed` (see catcoms-net); were that ever relaxed to
    // anonymous authorship, the result would route to a forwarding neighbour and joins would stall.
    fn on_add_request(&mut self, from_admin: PeerId, data: &[u8]) {
        if !self.group.is_designated_committer(&self.device) {
            return; // some other node is the serializer
        }
        let (invite_bytes, kp_bytes, pubkey, ts, sig) = match decode_add_request(data) {
            Ok(r) => r,
            Err(_) => return,
        };
        let Ok(invite) = InviteToken::decode(&invite_bytes) else {
            return;
        };
        let requester = DeviceId::from_public_key_bytes(&pubkey);

        // The requester must be a current member, fresh, and have signed the request binding the
        // exact KeyPackage (recompute its hash so a relay can't swap the KP under a valid sig).
        if !self.group.contains_device(&requester)
            || self.clock.now_ms().abs_diff(ts) > MAX_REQUEST_AGE_MS
        {
            self.stats.requests_rejected += 1;
            return;
        }
        let kp_hash = Cid::of(&kp_bytes);
        let transcript = add_req_transcript(
            &self.group.group_id(),
            &invite.invite_nonce,
            kp_hash.as_bytes(),
            &pubkey,
            ts,
        );
        if !verify_with_public_bytes(&pubkey, &transcript, &sig) {
            self.stats.requests_rejected += 1;
            return;
        }
        // THE ROLE RE-CHECK: the requester must BE the invite's named inviter, and that inviter
        // must be authorized (owner/admin) RIGHT NOW per the live roles doc. The invite must also
        // self-authenticate + target this group.
        if requester != invite.inviter_device_id
            || !self.inviter_is_authorized(&invite.inviter_device_id)
            || invite.group_id != self.group.group_id()
            || !invite.verify_self()
        {
            self.stats.requests_rejected += 1;
            return;
        }

        // Already admitted this nonce (the admin missed our result and retransmitted): re-deliver
        // the cached result instead of re-admitting (the ledger has consumed the nonce).
        if let Some(cached) = self.admit_results.get(&invite.invite_nonce) {
            let payload = encode_admit_result(
                &invite.invite_nonce,
                &cached.welcome,
                &cached.sealed_routing,
                &cached.owner_sig,
            );
            self.admit_result_outbox.push((from_admin, payload));
            return;
        }

        // Ledger fresh + bind the KeyPackage (reject junk before the heavy parse pays off).
        let now = self.clock.now_ms();
        if self.ledger.check(&invite, now).is_err() {
            return;
        }
        let Ok(key_package) = self.device.parse_key_package(&kp_bytes) else {
            return;
        };
        if self
            .group
            .validate_invite_binding(&key_package, &invite)
            .is_err()
        {
            return;
        }

        // Dedup on nonce + bound the queue (drop-oldest).
        if self
            .add_request_queue
            .iter()
            .any(|p| p.invite.invite_nonce == invite.invite_nonce)
        {
            return;
        }
        while self.add_request_queue.len() >= MAX_ADD_REQUESTS {
            self.add_request_queue.pop_front();
        }
        self.add_request_queue.push_back(PendingAdd {
            invite,
            kp_bytes,
            admin: from_admin,
        });
    }

    /// Owner side: admit each queued Add-request — run the MLS Add, cache the result, and push it
    /// back to the requesting admin (who re-signs + relays the Welcome to the joiner).
    fn drain_add_request_queue(&mut self) {
        let queued = std::mem::take(&mut self.add_request_queue);
        for p in queued {
            let now = self.clock.now_ms();
            let Some((welcome, sealed_routing, owner_sig)) =
                self.admit_now(&p.invite, &p.kp_bytes, now)
            else {
                continue;
            };
            let nonce = p.invite.invite_nonce;
            // Cache (bounded) so a later retransmit re-delivers without re-admitting.
            while self.admit_results.len() >= MAX_ADD_REQUESTS {
                let Some(victim) = self.admit_results.keys().next().copied() else {
                    break;
                };
                self.admit_results.remove(&victim);
            }
            self.admit_results.insert(
                nonce,
                CachedAdmit {
                    welcome: welcome.clone(),
                    sealed_routing: sealed_routing.clone(),
                    owner_sig,
                },
            );
            let payload = encode_admit_result(&nonce, &welcome, &sealed_routing, &owner_sig);
            self.admit_result_outbox.push((p.admin, payload));
        }
    }

    /// Admin side: the owner delivered a finalized admission. Verify the owner's signature, then
    /// re-sign the join transcript with our own (the inviter's) key — so the joiner's
    /// verification against `invite.inviter_public_key` is unchanged — and push the Welcome to the
    /// waiting joiner.
    fn on_admit_result(&mut self, data: &[u8]) {
        let Ok((nonce, welcome, sealed_routing, owner_sig)) = decode_admit_result(data) else {
            return;
        };
        if !self.outgoing_add_requests.contains_key(&nonce) {
            return; // not ours / already completed
        }
        // Verify the owner really built this Welcome before we vouch for it: the owner is the
        // group's designated committer; its signature over the join transcript must verify.
        let transcript = join_transcript(&self.group.group_id(), &nonce, &welcome, &sealed_routing);
        let Some(owner_id) = self.group.designated_committer() else {
            return;
        };
        let Some(owner_key) = self.group.member_signature_key(&owner_id) else {
            return;
        };
        if !verify_with_public_bytes(&owner_key, &transcript, &owner_sig) {
            tracing::warn!("admit result with an invalid owner signature; ignored");
            return;
        }
        let Ok(admin_sig) = self.device.sign(&transcript) else {
            return;
        };
        if let Some(out) = self.outgoing_add_requests.remove(&nonce) {
            let payload = encode_welcome_push(&welcome, &admin_sig, &sealed_routing);
            self.welcome_outbox.push((out.joiner, payload));
        }
    }

    /// Owner side: push finalized admit results to the requesting admins over RR (best-effort;
    /// the admin re-broadcasts the request if its result doesn't arrive).
    async fn drain_admit_result_outbox(&mut self) {
        let pending = std::mem::take(&mut self.admit_result_outbox);
        for (admin, payload) in pending {
            let mut req = vec![KIND_ADMIT_RESULT];
            req.extend_from_slice(&payload);
            let _ = self
                .transport
                .request(admin, ProtocolId(RR_PROTOCOL), Bytes::from(req))
                .await;
        }
    }

    /// Build the signed [`CommitRecord`] for an Add produced via the synchronous
    /// (already-merged) path.
    fn sign_add_record(
        &self,
        commit_epoch: u64,
        mls_commit: &[u8],
        base_authenticator: [u8; 32],
    ) -> CommitRecord {
        let committer_device = *self.device.device_id().as_bytes();
        let auth = commit_auth_transcript(
            &self.group.group_id(),
            commit_epoch,
            &base_authenticator,
            &committer_device,
            mls_commit,
        );
        let committer_sig = self.device.sign(&auth).expect("sign own commit");
        CommitRecord {
            group_id: self.group.group_id(),
            commit_epoch,
            committer_device,
            mls_commit: mls_commit.to_vec(),
            base_authenticator,
            committer_sig,
        }
    }
}

/// Join a server from a pasted [`InviteToken`]: authenticate the invite, mint a
/// bound KeyPackage, send it to `inviter` over the transport, and join from the
/// returned Welcome — but only after verifying the Welcome was **signed by the
/// inviter** named in the invite and that the joined group's id matches. This
/// defeats a malicious inviter/relay trying to divert the joiner into a group it
/// controls (the joiner's KeyPackage alone is not a secret). The caller must
/// already be transport-connected to `inviter` (via the invite's bootstrap
/// addresses).
pub async fn request_join<T: MeshTransport>(
    transport: &T,
    inviter: PeerId,
    device: &MlsDevice,
    invite: &InviteToken,
) -> Result<(ServerGroup, RoutingState), SyncError> {
    // Authenticate the pasted invite itself (signature + pubkey binds device id).
    if !invite.verify_self() {
        tracing::warn!("invite failed self-verification");
        return Err(SyncError::JoinRejected);
    }

    let key_package = device.key_package_for_invite(&invite.group_id, invite.invite_nonce)?;
    let kp_bytes = serialize_key_package(&key_package)?;

    let mut payload = vec![KIND_JOIN];
    payload.extend_from_slice(&encode_join_req(invite, &kp_bytes));

    tracing::debug!("sending join request");
    let resp = transport
        .request(inviter, ProtocolId(RR_PROTOCOL), Bytes::from(payload))
        .await?;
    match resp.split_first() {
        // Synchronous admission (single-committer): the Welcome is inline.
        Some((&JOIN_READY, rest)) => {
            let (welcome, signature, sealed_routing) = decode_join_resp(rest)?;
            finish_join(device, invite, &welcome, &signature, &sealed_routing)
        }
        // Staged admission (concurrent committers): the inviter will *push* the
        // signed Welcome once its staged commit wins and merges. Await it.
        Some((&JOIN_PENDING, _)) => {
            tracing::debug!("admission staged; awaiting the Welcome push");
            // The wall-clock bound on this wait lives at the call site (catcoms-app, which owns
            // the tokio runtime) so this crate stays runtime-agnostic — see the `request_join`
            // timeout wrapper there, so a never-finalizing owner can't wedge the joiner.
            await_welcome_push(transport, device, invite).await
        }
        // Empty or unknown => rejected.
        _ => Err(SyncError::JoinRejected),
    }
}

/// Verify a Welcome was signed by the invite's inviter and join from it.
fn finish_join(
    device: &MlsDevice,
    invite: &InviteToken,
    welcome: &[u8],
    signature: &[u8; 64],
    sealed_routing: &[u8],
) -> Result<(ServerGroup, RoutingState), SyncError> {
    let transcript = join_transcript(
        &invite.group_id,
        &invite.invite_nonce,
        welcome,
        sealed_routing,
    );
    if !invite.verify_inviter_signature(&transcript, signature) {
        tracing::warn!("Welcome was not signed by the invite's inviter (or transfer tampered)");
        return Err(SyncError::JoinRejected);
    }
    let group = ServerGroup::join(device, welcome)?;
    // Defense in depth: we must have landed in the group the invite named.
    if group.group_id() != invite.group_id {
        tracing::warn!("joined group id does not match the invite");
        return Err(SyncError::JoinRejected);
    }
    // We are now at the post-join epoch the inviter sealed the routing state under,
    // so we can open it and adopt the group's routing label/secrets. A present but
    // unopenable transfer (signature already verified) is a hard error.
    let routing = open_routing_transfer(&group, device, sealed_routing)?;
    tracing::info!(epoch = group.epoch(), "joined server via invite");
    Ok((group, routing))
}

/// Await the committer's provisional-Welcome push (`KIND_WELCOME`) for a staged
/// admission: an empty body means the committer lost its fork (the join is
/// rejected; the caller retries), otherwise it carries the signed Welcome.
async fn await_welcome_push<T: MeshTransport>(
    transport: &T,
    device: &MlsDevice,
    invite: &InviteToken,
) -> Result<(ServerGroup, RoutingState), SyncError> {
    loop {
        match transport.next_event().await {
            Some(TransportEvent::Request {
                data, responder, ..
            }) if data.first() == Some(&KIND_WELCOME) => {
                responder.respond(Bytes::new()); // ack the push
                match data[1..].split_first() {
                    Some((&JOIN_READY, body)) => {
                        let (welcome, signature, sealed_routing) = decode_join_resp(body)?;
                        return finish_join(device, invite, &welcome, &signature, &sealed_routing);
                    }
                    // Empty body => the committer lost; rejected.
                    _ => return Err(SyncError::JoinRejected),
                }
            }
            // Ignore unrelated requests (ack so the sender isn't left hanging).
            Some(TransportEvent::Request { responder, .. }) => responder.respond(Bytes::new()),
            Some(_) => continue,
            None => return Err(SyncError::JoinRejected),
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> fmt::Debug for ChannelSync<T, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChannelSync")
            .field("device", &self.device.device_id())
            .field("epoch", &self.group.epoch())
            .field("open_docs", &self.docs.len())
            .field("pending_commits", &self.pending_commits.len())
            .field("past_keys", &self.past_keys.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::{Hub, ManualClock, MemNetwork};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn catchup_request_roundtrips_through_codec() {
        let bytes = encode_catchup_req(DocType::Wiki, 99);
        assert_eq!(decode_catchup_req(&bytes).unwrap(), (DocType::Wiki, 99));
    }

    #[test]
    fn add_request_and_admit_result_round_trip_through_codecs() {
        // Add-request: invite ‖ key_package ‖ requester_pubkey ‖ ts ‖ sig
        let invite = b"invite-token-bytes".to_vec();
        let kp = b"key-package-bytes".to_vec();
        let pubkey = vec![7u8; 32];
        let ts = 1_234_567u64;
        let sig = [9u8; 64];
        let enc = encode_add_request(&invite, &kp, &pubkey, ts, &sig);
        assert_eq!(
            decode_add_request(&enc).unwrap(),
            (invite, kp, pubkey, ts, sig)
        );

        // Admit-result: invite_nonce ‖ welcome ‖ sealed_routing ‖ owner_sig
        let nonce = [3u8; 16];
        let welcome = b"welcome-bytes".to_vec();
        let routing = b"sealed-routing".to_vec();
        let owner_sig = [5u8; 64];
        let enc2 = encode_admit_result(&nonce, &welcome, &routing, &owner_sig);
        assert_eq!(
            decode_admit_result(&enc2).unwrap(),
            (nonce, welcome, routing, owner_sig)
        );

        // The Add-request transcript is deterministic + domain/group-bound.
        let kp_hash = [1u8; 32];
        let t1 = add_req_transcript(b"gid", &nonce, &kp_hash, &[7u8; 32], ts);
        assert_eq!(
            t1,
            add_req_transcript(b"gid", &nonce, &kp_hash, &[7u8; 32], ts)
        );
        assert_ne!(
            t1,
            add_req_transcript(b"gXd", &nonce, &kp_hash, &[7u8; 32], ts),
            "the transcript binds the group id"
        );
    }

    #[test]
    fn commit_catchup_request_roundtrips_through_codec() {
        let bytes = encode_commit_catchup_req(42);
        assert_eq!(decode_commit_catchup_req(&bytes).unwrap(), 42);
    }

    #[test]
    fn topics_are_keyed_by_the_routing_secret_and_bound_to_the_label() {
        let secret = [7u8; 32];
        let gid = b"group-id-123";
        let ctrl = control_topic(&secret, gid, 0);
        // A1: the topic depends on ns_secret (member-only), so an invite-holder who
        // knows only group_id computes a different value and cannot derive it.
        assert_ne!(ctrl, control_topic(&[8u8; 32], gid, 0));
        // Bound to the routing label, so it rotates on each removal.
        assert_ne!(ctrl, control_topic(&secret, gid, 1));
        // Channel topics: keyed, label-bound, per-doc separated, and never equal to
        // the (domain-separated) control topic.
        let ch = channel_topic(&secret, gid, DocType::Channel, 1, 0);
        assert_ne!(ch, ctrl);
        assert_ne!(ch, channel_topic(&[8u8; 32], gid, DocType::Channel, 1, 0));
        assert_ne!(ch, channel_topic(&secret, gid, DocType::Channel, 1, 1));
        assert_ne!(ch, channel_topic(&secret, gid, DocType::Channel, 2, 0));
        assert_ne!(ch, channel_topic(&secret, gid, DocType::Wiki, 1, 0));
    }

    #[test]
    fn bundle_decoders_reject_an_absurd_element_count() {
        // A forged header claiming a huge element count must be rejected up front,
        // not drive an allocation loop (defense in depth behind the response cap).
        let mut e = Encoder::new();
        e.put_u32(u32::MAX);
        let forged = e.finish();
        assert!(matches!(
            decode_commit_bundle(&forged),
            Err(SyncError::Malformed)
        ));
        assert!(matches!(decode_bundle(&forged), Err(SyncError::Malformed)));
    }

    #[test]
    fn commit_bundle_roundtrips_through_codec() {
        let records = vec![
            CommitRecord {
                group_id: b"gid".to_vec(),
                commit_epoch: 1,
                committer_device: [3u8; 32],
                mls_commit: vec![9, 9, 9],
                base_authenticator: [5u8; 32],
                committer_sig: [6u8; 64],
            },
            CommitRecord {
                group_id: b"gid".to_vec(),
                commit_epoch: 2,
                committer_device: [4u8; 32],
                mls_commit: vec![1, 2, 3, 4],
                base_authenticator: [7u8; 32],
                committer_sig: [8u8; 64],
            },
        ];
        let decoded = decode_commit_bundle(&encode_commit_bundle(&records)).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].commit_epoch, 1);
        assert_eq!(decoded[1].mls_commit, vec![1, 2, 3, 4]);
    }

    /// Frame a commit record as a control-topic message (tagged envelope).
    fn framed_commit(r: &CommitRecord) -> Vec<u8> {
        let mut v = vec![CTRL_COMMIT];
        v.extend_from_slice(&r.encode());
        v
    }

    /// A genuine membership commit applied on the inbound path is accepted; a copy
    /// with a tampered committer (or signature) is rejected — committer
    /// authorization is verified by signature on apply, not only at admission.
    #[tokio::test]
    async fn control_commit_with_a_forged_committer_label_is_rejected() {
        let hub = Hub::new();
        let alice = MlsDevice::generate().unwrap();
        let alice_group = ServerGroup::create(&alice).unwrap();
        let alice_peer = PeerId::from_u64(1);
        let mut asy = ChannelSync::new(
            hub.join(alice_peer),
            alice_group,
            alice,
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
        );

        // Bob joins (Alice 0->1). Bob starts at epoch 1.
        let bob = MlsDevice::generate().unwrap();
        let invite_b = asy.mint_invite([1u8; 16], 10_000, vec![]).unwrap();
        let bob_net = hub.join(PeerId::from_u64(2));
        let (bob_joined, _) = tokio::join!(
            request_join(&bob_net, alice_peer, &bob, &invite_b),
            asy.run_once(),
        );
        let (bob_group, bob_routing) = bob_joined.unwrap();
        let mut bsy = ChannelSync::new_joined(
            bob_net,
            bob_group,
            bob,
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            bob_routing,
        );

        // Carol joins (Alice 1->2): Alice produces a genuine commit at epoch 1.
        let carol = MlsDevice::generate().unwrap();
        let invite_c = asy.mint_invite([2u8; 16], 10_000, vec![]).unwrap();
        let carol_net = hub.join(PeerId::from_u64(3));
        let (_carol, _) = tokio::join!(
            request_join(&carol_net, alice_peer, &carol, &invite_c),
            asy.run_once(),
        );
        let genuine = asy
            .commit_log
            .iter()
            .find(|r| r.commit_epoch == 1)
            .expect("Alice retained the epoch-1 commit")
            .clone();

        // A tampered copy claiming a different committer is rejected; Bob stays put.
        let mut forged = genuine.clone();
        forged.committer_device = [0xAA; 32];
        bsy.on_control(PeerId::from_u64(0), &framed_commit(&forged));
        assert_eq!(bsy.epoch(), 1, "forged-committer commit must be rejected");
        assert_eq!(bsy.stats().commits_applied, 0);

        // A copy with a tampered signature (real committer) is also rejected.
        let mut bad_sig = genuine.clone();
        bad_sig.committer_sig[0] ^= 0xFF;
        bsy.on_control(PeerId::from_u64(0), &framed_commit(&bad_sig));
        assert_eq!(bsy.epoch(), 1, "bad-signature commit must be rejected");

        // The genuine commit (committer = Alice, valid signature) applies.
        bsy.on_control(PeerId::from_u64(0), &framed_commit(&genuine));
        assert_eq!(bsy.epoch(), 2, "genuine commit applies");
    }

    /// A single-member sync node (no peers) — enough to exercise the buffering
    /// bounds, which are pure local logic.
    fn solo_node() -> ChannelSync<MemNetwork, ChaCha20Rng> {
        let alice = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&alice).unwrap();
        let hub = Hub::new();
        ChannelSync::new(
            hub.join(PeerId::from_u64(1)),
            group,
            alice,
            ChaCha20Rng::seed_from_u64(0),
            Box::new(ManualClock::new(1_000)),
        )
    }

    #[tokio::test]
    async fn channel_sync_snapshot_round_trips_durable_state() {
        use automerge::{transaction::Transactable, ROOT};
        let mut node = solo_node();
        let gid = node.group.group_id();
        let epoch = node.group.epoch();

        node.open_channel(DocType::Channel, 1).await.unwrap();
        node.post(DocType::Channel, 1, |d| d.put(ROOT, "k", "v"))
            .await
            .unwrap();
        node.ledger.consume([7u8; 16]).unwrap();
        let ops = node.doc(DocType::Channel, 1).unwrap().op_count();
        let label = node.routing_label;
        let control_topic = node.control_topic.clone();

        // Snapshot, then restore onto a FRESH transport (connections don't persist).
        let snap = node.snapshot().unwrap();
        let hub = Hub::new();
        let restored = ChannelSync::restore(
            &snap,
            hub.join(PeerId::from_u64(2)),
            ChaCha20Rng::seed_from_u64(0),
            Box::new(ManualClock::new(1_000)),
        )
        .unwrap();

        // MLS group, documents, routing label, and the invite ledger all survive…
        assert_eq!(restored.group.group_id(), gid);
        assert_eq!(restored.group.epoch(), epoch);
        assert_eq!(restored.routing_label, label);
        // The restored node recomputes the SAME blinded control topic (routing survived).
        assert_eq!(restored.control_topic, control_topic);
        assert_eq!(restored.doc(DocType::Channel, 1).unwrap().op_count(), ops);
        assert!(restored.ledger.is_consumed(&[7u8; 16]));
        // …and a corrupt snapshot is rejected.
        assert!(ChannelSync::restore(
            b"garbage",
            hub.join(PeerId::from_u64(3)),
            ChaCha20Rng::seed_from_u64(0),
            Box::new(ManualClock::new(1_000)),
        )
        .is_err());
    }

    #[test]
    fn peer_addresses_are_extractable_from_a_snapshot_for_redial() {
        // 9g: a reloading node pulls its last-known peer multiaddrs out of the snapshot to
        // dial them at transport construction — without a full restore.
        let mut node = solo_node();
        node.peer_records.insert(
            DeviceId::from_bytes([5u8; 32]),
            PeerDescriptor {
                device_pubkey: vec![5u8; 32],
                peer_id: [1u8; 32],
                addresses: vec!["/ip4/1.2.3.4/tcp/9/p2p/X".to_string()],
                seq: 1,
                signature: [0u8; 64],
            },
        );
        let snap = node.snapshot().unwrap();
        assert_eq!(
            peer_addrs_from_snapshot(&snap).unwrap(),
            vec!["/ip4/1.2.3.4/tcp/9/p2p/X".to_string()]
        );

        // A node that knew no peers extracts cleanly to an empty list (not an error).
        let mut node2 = solo_node();
        assert!(peer_addrs_from_snapshot(&node2.snapshot().unwrap())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn the_join_transfer_carries_the_file_wrap_key() {
        // 9h: the transfer plaintext bundles the routing state AND the group file-wrap key.
        let secrets = vec![(0u64, [3u8; 32])];
        let fwk = [7u8; 32];
        let (label, got_secrets, got_fwk) =
            decode_join_transfer(&encode_join_transfer(5, &secrets, &fwk)).unwrap();
        assert_eq!(label, 5);
        assert_eq!(got_secrets, secrets);
        assert_eq!(got_fwk, fwk);
    }

    #[test]
    fn files_seal_open_under_the_group_key_and_the_key_survives_a_snapshot() {
        // 9h: a file sealed under the group file-wrap key opens back; the key persists across
        // a snapshot (so a reloaded node still opens files); a different key cannot open it.
        let mut node = solo_node();
        assert!(node.has_file_key());
        let (fref, stored) = node.seal_file(b"secret document", "text/plain").unwrap();
        assert_eq!(node.open_file(&stored, &fref).unwrap(), b"secret document");

        // Snapshot → restore preserves the file-wrap key.
        let snap = node.snapshot().unwrap();
        let hub = Hub::new();
        let restored = ChannelSync::restore(
            &snap,
            hub.join(PeerId::from_u64(9)),
            ChaCha20Rng::seed_from_u64(0),
            Box::new(ManualClock::new(1_000)),
        )
        .unwrap();
        assert_eq!(
            restored.open_file(&stored, &fref).unwrap(),
            b"secret document"
        );

        // A node holding a different key cannot open it.
        let mut other = solo_node();
        other.file_wrap_key = Zeroizing::new([1u8; 32]);
        assert!(other.open_file(&stored, &fref).is_err());
    }

    #[test]
    fn adopting_a_transfer_installs_the_file_wrap_key_so_a_joiner_opens_the_founders_file() {
        // 9h end-to-end (local): a founder seals a file; a joiner that adopts the routing
        // transfer (which now carries the founder's file-wrap key) can open it.
        let mut founder = solo_node();
        let (fref, stored) = founder.seal_file(b"hi from alice", "text/plain").unwrap();
        let founder_key = *founder.file_wrap_key;

        let mut joiner = solo_node();
        joiner.file_wrap_key = Zeroizing::new([0u8; 32]); // pretend it has no key yet
        assert!(joiner.open_file(&stored, &fref).is_err());

        joiner.adopt_routing_state(RoutingState {
            label: 0,
            secrets: Vec::new(), // even with no routing secrets, the key is installed
            file_wrap_key: Some(founder_key),
        });
        assert_eq!(
            joiner.open_file(&stored, &fref).unwrap(),
            b"hi from alice",
            "the joiner opens the founder's file with the transferred key"
        );
    }

    #[test]
    fn signed_commit_response_roundtrips_through_codec() {
        let bundle = b"opaque-bundle-bytes".to_vec();
        let pubkey = vec![7u8; 32];
        let sig = [9u8; 64];
        let (p, s, b) =
            decode_signed_commit_resp(&encode_signed_commit_resp(&pubkey, &sig, &bundle)).unwrap();
        assert_eq!(p, pubkey);
        assert_eq!(s, sig);
        assert_eq!(b, bundle);
    }

    #[test]
    fn a_commit_catchup_response_verifies_only_from_a_current_member_bound_to_the_request() {
        // 6e-3d-5: the Sybil-C1 gate — a served catch-up bundle is trusted only if a
        // current member signed it for THIS request.
        let alice = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&alice).unwrap();
        let mallory = MlsDevice::generate().unwrap(); // not in the group

        let gid = group.group_id();
        let requester_pubkey = vec![1u8; 32];
        let req_ts = 1_234u64;
        let nonce = [9u8; 16];
        let req_epoch = 3u64;
        let bundle = b"a-bundle".to_vec();
        let transcript =
            catchup_resp_transcript(&gid, &requester_pubkey, req_ts, &nonce, req_epoch, &bundle);

        // Accepted: Alice is a member and her signature over this transcript verifies.
        let alice_sig = alice.sign(&transcript).unwrap();
        assert!(group.contains_device(&alice.device_id()));
        assert!(verify_with_public_bytes(
            &alice.public_key_bytes(),
            &transcript,
            &alice_sig
        ));

        // Rejected: Mallory's signature is cryptographically valid, but she is NOT in
        // the roster — the membership check (contains_device) drops it.
        let mallory_sig = mallory.sign(&transcript).unwrap();
        assert!(verify_with_public_bytes(
            &mallory.public_key_bytes(),
            &transcript,
            &mallory_sig
        ));
        assert!(!group.contains_device(&mallory.device_id()));

        // Rejected: a tampered bundle breaks the signature.
        let tampered = catchup_resp_transcript(
            &gid,
            &requester_pubkey,
            req_ts,
            &nonce,
            req_epoch,
            b"tampered",
        );
        assert!(!verify_with_public_bytes(
            &alice.public_key_bytes(),
            &tampered,
            &alice_sig
        ));

        // Rejected: replaying the reply against a different request (different ts)
        // breaks the binding, so a captured response cannot be reused.
        let other_ts = catchup_resp_transcript(
            &gid,
            &requester_pubkey,
            req_ts + 1,
            &nonce,
            req_epoch,
            &bundle,
        );
        assert!(!verify_with_public_bytes(
            &alice.public_key_bytes(),
            &other_ts,
            &alice_sig
        ));

        // Rejected: a DIFFERENT nonce (same ts) — the 6e-3d-6 anti-replay bind that
        // closes the same-millisecond `ts`-collision window — also breaks the binding.
        let other_nonce = catchup_resp_transcript(
            &gid,
            &requester_pubkey,
            req_ts,
            &[10u8; 16],
            req_epoch,
            &bundle,
        );
        assert!(!verify_with_public_bytes(
            &alice.public_key_bytes(),
            &other_nonce,
            &alice_sig
        ));

        // Rejected: a different requester epoch breaks the binding too.
        let other_epoch = catchup_resp_transcript(
            &gid,
            &requester_pubkey,
            req_ts,
            &nonce,
            req_epoch + 1,
            &bundle,
        );
        assert!(!verify_with_public_bytes(
            &alice.public_key_bytes(),
            &other_epoch,
            &alice_sig
        ));
    }

    #[test]
    fn an_authed_request_roundtrips_with_its_nonce_and_epoch() {
        // The framed request carries the nonce + req_epoch verbatim so the verifier
        // can reconstruct the exact transcript (6e-3d-6).
        let inner = b"inner-body".to_vec();
        let pubkey = vec![2u8; 32];
        let ts = 77u64;
        let nonce = [5u8; 16];
        let epoch = 9u64;
        let sig = [3u8; 64];
        let framed = encode_authed_request(&inner, &pubkey, ts, &nonce, epoch, &sig);
        let (i, p, t, n, e, s) = decode_authed_request(&framed).unwrap();
        assert_eq!((i, p, t, n, e, s), (inner, pubkey, ts, nonce, epoch, sig));
    }

    #[test]
    fn pick_catchup_peer_prefers_a_proven_member_over_an_untrusted_candidate() {
        let mut node = solo_node();
        node.remember_peer(PeerId::from_u64(1)); // untrusted candidate
        node.promote_member_peer(PeerId::from_u64(2)); // proven member
        assert_eq!(
            node.pick_catchup_peer(),
            Some(PeerId::from_u64(2)),
            "a proven member is preferred over a candidate"
        );
        // With the member failed, fall back to the candidate (bootstrap is preserved).
        node.note_failed_catchup_peer(PeerId::from_u64(2));
        assert_eq!(node.pick_catchup_peer(), Some(PeerId::from_u64(1)));
    }

    #[test]
    fn a_demoted_member_is_no_longer_preferred() {
        // A member that was promoted but has since been removed from the roster is
        // demoted (review fix), so it stops front-running honest sources every gap.
        let mut node = solo_node();
        node.remember_peer(PeerId::from_u64(1)); // candidate
        node.promote_member_peer(PeerId::from_u64(2)); // proven member
        assert_eq!(node.pick_catchup_peer(), Some(PeerId::from_u64(2)));
        node.demote_member_peer(PeerId::from_u64(2));
        assert_eq!(
            node.pick_catchup_peer(),
            Some(PeerId::from_u64(1)),
            "a demoted ex-member is no longer preferred"
        );
    }

    #[test]
    fn join_namespace_is_invite_derivable_and_per_rendezvous() {
        // 6e-3d-9: a joiner derives this from the invite alone (no group secret), and
        // it matches what the inviter registers under.
        let gid = b"group-id-abc";
        let nonce = [3u8; 16];
        let (rz1, rz2) = (b"rendezvous-one", b"rendezvous-two");
        let ns1 = join_namespace(gid, &nonce, rz1);
        assert_eq!(ns1, join_namespace(gid, &nonce, rz1), "deterministic");
        // Per-rendezvous diversification + binding to nonce and group.
        assert_ne!(ns1, join_namespace(gid, &nonce, rz2));
        assert_ne!(ns1, join_namespace(gid, &[4u8; 16], rz1));
        assert_ne!(ns1, join_namespace(b"other-group", &nonce, rz1));
        // Well-formed, short namespace (well under the 255-byte rendezvous cap).
        assert!(ns1.starts_with("catcoms1-"));
        assert!(ns1.len() <= 255);
    }

    #[test]
    fn a_membership_tag_binds_the_secret_label_and_peer() {
        // 6e-3d-6 pre-dial tag: only a member (holding ns_secret_L) can produce a tag
        // that verifies, and it is bound to the registrant's peer id + seq.
        let node = solo_node();
        let rz = b"rendezvous-peer-id-bytes";
        let namespaces = node.rendezvous_namespaces(rz);
        let namespace = namespaces.first().expect("a namespace at L=0").clone();
        let peer = b"member-libp2p-peer-id";
        let seq = 5u64;
        let tag = node.membership_tag(rz, 0, peer, seq).expect("tag at L=0");

        // The member that produced it verifies its own tag.
        assert!(node.verify_membership_tag(rz, &namespace, peer, seq, &tag));
        // Bound to the peer id: a different peer id does not verify (defeats a
        // colluding rendezvous grafting a real tag onto a Sybil record).
        assert!(!node.verify_membership_tag(rz, &namespace, b"a-different-peer", seq, &tag));
        // Bound to the record seq.
        assert!(!node.verify_membership_tag(rz, &namespace, peer, seq + 1, &tag));
        // A flipped tag bit fails (constant-time compare still rejects).
        let mut bad = tag;
        bad[0] ^= 0x01;
        assert!(!node.verify_membership_tag(rz, &namespace, peer, seq, &bad));
        // A namespace this node does not recognise fails closed (no dial on junk).
        assert!(!node.verify_membership_tag(rz, "catcoms1-0000000000", peer, seq, &tag));

        // A DIFFERENT group (different ns_secret) cannot verify this tag, even handed
        // the namespace string — it derives a different namespace, so it doesn't even
        // recognise this one (fails closed), and could never recompute the tag.
        let other = solo_node();
        assert!(!other.verify_membership_tag(rz, &namespace, peer, seq, &tag));
    }

    #[test]
    fn membership_tag_preimage_is_canonical_across_field_boundaries() {
        // Length-prefixing makes the rz_peer ‖ peer_id boundary unambiguous: shifting a
        // byte from one field to the other yields a different tag, so a colluding
        // rendezvous cannot graft a real member's tag onto a Sybil by choosing a
        // boundary-confusable (rz_peer, peer_id) split.
        let secret = [3u8; 32];
        let gid = b"group-id";
        // rz_peer and peer_id are the only two adjacent variable-length fields, so they
        // are the boundary a colluding rendezvous would target. (rz_peer="AB",
        // peer_id="CD") and (rz_peer="A", peer_id="BCD") share an identical raw
        // concatenation but are distinct fields — length-prefixing keeps the tags
        // distinct (a raw-concat hash would collide them).
        let a = routing_membership_tag(&secret, gid, b"AB", 0, b"CD", 7);
        let b = routing_membership_tag(&secret, gid, b"A", 0, b"BCD", 7);
        assert_ne!(a, b, "rz_peer/peer_id boundary must be unambiguous");
    }

    #[test]
    fn future_commit_buffer_is_bounded_and_drops_furthest_first() {
        let mut node = solo_node();
        node.set_config(SyncConfig {
            max_pending_commits: 4,
            max_commit_gap: 1_000,
            ..SyncConfig::default()
        });
        let gid = node.group.group_id();

        // Buffer many future commits (epoch 0 is current; these are all ahead).
        for epoch in 1..=50u64 {
            node.buffer_future_commit(CommitRecord {
                group_id: gid.clone(),
                commit_epoch: epoch,
                committer_device: [0u8; 32],
                mls_commit: vec![epoch as u8],
                base_authenticator: [0u8; 32],
                committer_sig: [0u8; 64],
            });
        }
        assert_eq!(node.pending_commits.len(), 4, "buffer must stay capped");
        // The nearest (gap-filling) epochs are kept; the furthest are dropped.
        let kept: Vec<u64> = node.pending_commits.keys().copied().collect();
        assert_eq!(kept, vec![1, 2, 3, 4]);
    }

    #[test]
    fn commit_beyond_the_gap_bound_is_not_buffered() {
        let mut node = solo_node();
        node.set_config(SyncConfig {
            max_commit_gap: 100,
            ..SyncConfig::default()
        });
        let gid = node.group.group_id();
        node.buffer_future_commit(CommitRecord {
            group_id: gid,
            commit_epoch: 10_000, // far beyond current(0) + 100
            committer_device: [0u8; 32],
            mls_commit: vec![1],
            base_authenticator: [0u8; 32],
            committer_sig: [0u8; 64],
        });
        assert!(
            node.pending_commits.is_empty(),
            "a far-future commit must be rejected, not buffered"
        );
    }

    #[test]
    fn known_peers_are_bounded_and_most_recent_wins() {
        let mut node = solo_node();
        node.set_config(SyncConfig {
            max_known_peers: 3,
            ..SyncConfig::default()
        });
        for n in 0..10u64 {
            node.remember_peer(PeerId::from_u64(n));
        }
        assert_eq!(node.known_peers.len(), 3);
        // The most-recently-seen peer is chosen for catch-up.
        assert_eq!(node.pick_catchup_peer(), Some(PeerId::from_u64(9)));
    }

    // --- member PEX (6e-3d-7) ------------------------------------------------

    type Member = ChannelSync<MemNetwork, ChaCha20Rng>;

    /// Build a converged `n`-member group over one hub: `members[0]` is the founder
    /// (full roster), each subsequent member joins in turn (so the **last** joiner's
    /// Welcome carries the full roster). Returns the members and their device ids,
    /// aligned by index.
    async fn build_members(n: u64) -> (std::sync::Arc<Hub>, Vec<Member>, Vec<DeviceId>) {
        assert!(n >= 1);
        let hub = Hub::new();
        let founder = MlsDevice::generate().unwrap();
        let founder_id = founder.device_id();
        let fgroup = ServerGroup::create(&founder).unwrap();
        let fpeer = PeerId::from_u64(1);
        let mut founder_sync = ChannelSync::new(
            hub.join(fpeer),
            fgroup,
            founder,
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
        );
        let mut members = Vec::new();
        let mut ids = vec![founder_id];
        for i in 2..=n {
            let dev = MlsDevice::generate().unwrap();
            ids.push(dev.device_id());
            let invite = founder_sync
                .mint_invite([i as u8; 16], 10_000, vec![])
                .unwrap();
            let net = hub.join(PeerId::from_u64(i));
            let (joined, _) = tokio::join!(
                request_join(&net, fpeer, &dev, &invite),
                founder_sync.run_once(),
            );
            let (group, routing) = joined.unwrap();
            members.push(ChannelSync::new_joined(
                net,
                group,
                dev,
                ChaCha20Rng::seed_from_u64(i),
                Box::new(ManualClock::new(1_000)),
                routing,
            ));
        }
        members.insert(0, founder_sync);
        (hub, members, ids)
    }

    #[tokio::test]
    async fn the_committer_rejects_a_remove_request_forged_by_a_non_owner() {
        // Removal is owner-only (THREAT-MODEL R1). A modified non-owner client could craft a
        // well-formed, correctly-self-signed remove request and broadcast it; the committer
        // (owner) must still reject it, because the requester is not the designated committer.
        let (_hub, mut members, ids) = build_members(3).await;
        let target = ids[2];

        // Forge a valid request *from the non-owner* (members[1]) — correct signature, fresh ts.
        let (pubkey, ts, signature) = {
            let bob = &members[1];
            let pubkey = bob.device.public_key_bytes();
            let ts = bob.clock.now_ms();
            let transcript =
                remove_req_transcript(&bob.group.group_id(), target.as_bytes(), &pubkey, ts);
            let signature = bob.device.sign(&transcript).unwrap();
            (pubkey, ts, signature)
        };
        let body = encode_remove_request(target.as_bytes(), &pubkey, ts, &signature);

        let alice = &mut members[0]; // the owner / designated committer
        let epoch_before = alice.epoch();
        let rejected_before = alice.stats.requests_rejected;
        alice.on_remove_request(&body);

        assert_eq!(
            alice.epoch(),
            epoch_before,
            "a non-owner request causes no commit"
        );
        assert!(
            alice.contains_member(&target),
            "the target is still a member"
        );
        assert_eq!(
            alice.stats.requests_rejected,
            rejected_before + 1,
            "the forged request was counted as rejected"
        );
    }

    #[tokio::test]
    async fn inviter_is_authorized_tracks_the_owners_local_roster() {
        let (_hub, mut members, ids) = build_members(3).await;
        let owner = ids[0];
        let admin = ids[1];
        let plain = ids[2];
        let outsider = MlsDevice::generate().unwrap().device_id();

        // The owner grants members[1] admin via its authoritative roster.
        grant_admin(&mut members[0], &admin).await;

        assert!(
            members[0].inviter_is_authorized(&owner),
            "the owner is always authorized"
        );
        assert!(
            members[0].inviter_is_authorized(&admin),
            "a granted admin is authorized"
        );
        assert!(
            !members[0].inviter_is_authorized(&plain),
            "a plain member is not authorized"
        );
        assert!(
            !members[0].inviter_is_authorized(&outsider),
            "a non-member is not authorized"
        );

        // Revoking the admin removes it from the owner's gate.
        members[0]
            .set_admin(&fingerprint(&admin), false)
            .await
            .unwrap();
        assert!(
            !members[0].inviter_is_authorized(&admin),
            "a revoked admin is no longer authorized"
        );
    }

    #[tokio::test]
    async fn a_replayed_old_roster_does_not_re_authorize_a_demoted_admin() {
        // THREAT-MODEL item 3 — the core property. A demoted admin is still a member and can
        // re-publish its (still validly owner-signed) OLD roster into the shared CRDT. The
        // owner's admission gate reads its LOCAL authoritative roster, so the replay cannot
        // re-authorize the demoted admin, and a CTRL_ADD_REQUEST from it is rejected.
        let (_hub, mut members, ids) = build_members(2).await;
        let mallory = ids[1];
        let mallory_fp = fingerprint(&mallory);

        // Owner grants Mallory (gen 1); capture the gen-1 published roster value.
        grant_admin(&mut members[0], &mallory).await;
        assert!(members[0].inviter_is_authorized(&mallory), "granted");
        let stale_roster = published_roster_value(&members[0]);
        assert!(!stale_roster.is_empty(), "a roster was published");

        // Owner demotes Mallory (gen 2).
        members[0].set_admin(&mallory_fp, false).await.unwrap();
        assert!(!members[0].inviter_is_authorized(&mallory), "revoked");

        // Mallory replays the gen-1 roster value back into the CRDT (the owner's doc here, to
        // simulate it having propagated). It still verifies (owner-signed), so read_published
        // would show her — but the gate ignores the CRDT.
        {
            use automerge::{transaction::Transactable, ScalarValue, ROOT};
            let value = stale_roster.clone();
            members[0]
                .post(DocType::MemberRoles, ROLES_DOC, move |d| {
                    d.put(ROOT, ROSTER_KEY, ScalarValue::Bytes(value))
                })
                .await
                .unwrap();
        }
        // The published copy now (wrongly) names Mallory — proving the replay "succeeds" at the
        // CRDT layer — yet the owner's gate still rejects her.
        let published = members[0]
            .doc(DocType::MemberRoles, ROLES_DOC)
            .and_then(|d| read_published_roster(d.doc(), &members[0].group_id(), &ids[0]))
            .unwrap_or_default();
        assert!(
            published.contains(&mallory_fp),
            "the replayed CRDT copy does name Mallory (so the replay is real)"
        );
        assert!(
            !members[0].inviter_is_authorized(&mallory),
            "but the owner's local-roster gate is unaffected by the replay (item 3 closed)"
        );

        // And the full admission gate rejects a CTRL_ADD_REQUEST from the demoted admin.
        let invite = members[1].mint_invite([3u8; 16], u64::MAX, vec![]).unwrap();
        let dave = MlsDevice::generate().unwrap();
        let kp = dave
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let kp_bytes = serialize_key_package(&kp).unwrap();
        let body = admin_add_request_body(&members[1], &invite, &kp_bytes);
        let owner = &mut members[0];
        let epoch_before = owner.epoch();
        owner.on_add_request(PeerId::from_u64(2), &body);
        owner.drain_add_request_queue();
        assert_eq!(
            owner.epoch(),
            epoch_before,
            "the demoted admin's Add-request causes no admission"
        );
        assert!(
            !owner.contains_member(&dave.device_id()),
            "Dave was not admitted via a demoted admin"
        );
    }

    #[tokio::test]
    async fn the_admin_roster_survives_a_snapshot_restore() {
        // The owner-local authoritative roster is the security source of truth, so it must
        // persist. Grant an admin, snapshot+restore, and confirm the gate still authorizes it.
        let (_hub, mut members, ids) = build_members(2).await;
        let admin = ids[1];
        grant_admin(&mut members[0], &admin).await;
        let snap = members[0].snapshot().unwrap();

        let hub2 = Hub::new();
        let net = hub2.join(PeerId::from_u64(99));
        let restored = Member::restore(
            &snap,
            net,
            ChaCha20Rng::seed_from_u64(7),
            Box::new(ManualClock::new(1_000)),
        )
        .unwrap();
        assert!(
            restored.inviter_is_authorized(&admin),
            "the granted admin is still authorized after restore"
        );
        assert!(
            !restored.inviter_is_authorized(&MlsDevice::generate().unwrap().device_id()),
            "a non-admin is still rejected after restore"
        );
    }

    /// Grant `target` admin via the owner's authoritative roster (item 3) + publish the copy.
    async fn grant_admin(owner: &mut Member, target: &DeviceId) {
        owner
            .open_channel(DocType::MemberRoles, ROLES_DOC)
            .await
            .unwrap();
        owner.set_admin(&fingerprint(target), true).await.unwrap();
    }

    /// Read the raw `roster` value bytes from a member's roles doc (for replay tests).
    fn published_roster_value(m: &Member) -> Vec<u8> {
        use automerge::{ReadDoc, ScalarValue, Value, ROOT};
        let doc = m.doc(DocType::MemberRoles, ROLES_DOC).unwrap();
        match doc.doc().get(ROOT, ROSTER_KEY) {
            Ok(Some((Value::Scalar(s), _))) => match s.as_ref() {
                ScalarValue::Bytes(b) => b.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// Build the body an admin would broadcast in `CTRL_ADD_REQUEST` for `invite`/`kp_bytes`.
    fn admin_add_request_body(admin: &Member, invite: &InviteToken, kp_bytes: &[u8]) -> Vec<u8> {
        let pubkey = admin.device.public_key_bytes();
        let ts = admin.clock.now_ms();
        let kp_hash = Cid::of(kp_bytes);
        let transcript = add_req_transcript(
            &admin.group.group_id(),
            &invite.invite_nonce,
            kp_hash.as_bytes(),
            &pubkey,
            ts,
        );
        let sig = admin.device.sign(&transcript).unwrap();
        encode_add_request(&invite.encode(), kp_bytes, &pubkey, ts, &sig)
    }

    #[tokio::test]
    async fn the_owner_admits_a_valid_admin_add_request() {
        // Owner side of Option C: a valid Add-request from an authorized admin is admitted (the
        // owner commits the MLS Add and queues a result back to the admin).
        let (_hub, mut members, ids) = build_members(2).await;
        let admin = ids[1];
        grant_admin(&mut members[0], &admin).await;
        let invite = members[1].mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let dave = MlsDevice::generate().unwrap();
        let kp = dave
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let kp_bytes = serialize_key_package(&kp).unwrap();
        let body = admin_add_request_body(&members[1], &invite, &kp_bytes);

        let owner = &mut members[0];
        let epoch_before = owner.epoch();
        owner.on_add_request(PeerId::from_u64(2), &body);
        owner.drain_add_request_queue();

        assert_eq!(
            owner.epoch(),
            epoch_before + 1,
            "the owner committed the admission"
        );
        assert!(
            owner.contains_member(&dave.device_id()),
            "Dave was admitted"
        );
        assert_eq!(
            owner.admit_result_outbox.len(),
            1,
            "a result was queued back to the admin"
        );
    }

    #[tokio::test]
    async fn an_admin_relays_the_owner_admit_result_so_the_joiner_accepts() {
        // The full Option-C chain at the method level: admin requests → owner admits → admin
        // relays the Welcome, re-signed with ITS key, so the joiner's existing finish_join
        // verification (against the invite's inviter key) accepts it — the Welcome-auth chain
        // (Option B), validated without the multi-party network dance.
        let (_hub, mut members, ids) = build_members(2).await;
        let admin_id = ids[1];
        grant_admin(&mut members[0], &admin_id).await;
        let invite = members[1].mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let dave = MlsDevice::generate().unwrap();
        let kp = dave
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let kp_bytes = serialize_key_package(&kp).unwrap();
        let dave_peer = PeerId::from_u64(50);
        let now = members[1].clock.now_ms();

        // Admin issues the request (stores the OutgoingAdd so it can relay the result).
        members[1].request_add(invite.clone(), kp_bytes.clone(), dave_peer, now);

        // Owner admits it and produces the admit result.
        let body = admin_add_request_body(&members[1], &invite, &kp_bytes);
        members[0].on_add_request(PeerId::from_u64(2), &body);
        members[0].drain_add_request_queue();
        let result_body = members[0].admit_result_outbox.last().unwrap().1.clone();

        // Admin processes the result → re-signs + queues the Welcome to the joiner.
        members[1].on_admit_result(&result_body);
        assert_eq!(
            members[1].welcome_outbox.len(),
            1,
            "the admin relayed a Welcome to the joiner"
        );
        assert_eq!(
            members[1].welcome_outbox[0].0, dave_peer,
            "the Welcome targets the joiner"
        );

        // The relayed Welcome carries the ADMIN's signature, so the joiner's finish_join (which
        // verifies against invite.inviter_public_key == the admin's key) accepts it — the
        // no-substitution property of Option B.
        let payload = &members[1].welcome_outbox[0].1; // [JOIN_READY] ‖ encode_join_resp(...)
        let (welcome, sig, sealed_routing) = decode_join_resp(&payload[1..]).unwrap();
        let transcript = join_transcript(
            &invite.group_id,
            &invite.invite_nonce,
            &welcome,
            &sealed_routing,
        );
        assert!(
            invite.verify_inviter_signature(&transcript, &sig),
            "the relayed Welcome verifies against the invite's inviter (Option B)"
        );
    }

    #[tokio::test]
    async fn a_non_admin_add_request_is_rejected_by_the_owner() {
        // A plain member (no admin grant) requests an admission; the owner's on_add_request must
        // reject it at the role re-check — no commit, no admission.
        let (_hub, mut members, _ids) = build_members(3).await; // owner + two plain members
        let plain = &members[1];
        // Forge a well-formed, correctly-signed Add-request from the plain member.
        let invite = plain.mint_invite([8u8; 16], u64::MAX, vec![]).unwrap();
        let dave = MlsDevice::generate().unwrap();
        let kp = dave
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let kp_bytes = serialize_key_package(&kp).unwrap();
        let (pubkey, ts, sig) = {
            let p = &members[1];
            let pubkey = p.device.public_key_bytes();
            let ts = p.clock.now_ms();
            let kp_hash = Cid::of(&kp_bytes);
            let transcript = add_req_transcript(
                &p.group.group_id(),
                &invite.invite_nonce,
                kp_hash.as_bytes(),
                &pubkey,
                ts,
            );
            (pubkey, ts, p.device.sign(&transcript).unwrap())
        };
        let body = encode_add_request(&invite.encode(), &kp_bytes, &pubkey, ts, &sig);

        let owner = &mut members[0];
        let epoch_before = owner.epoch();
        let rejected_before = owner.stats.requests_rejected;
        owner.on_add_request(PeerId::from_u64(9), &body);
        owner.drain_add_request_queue();

        assert_eq!(owner.epoch(), epoch_before, "no admission was committed");
        assert!(
            !owner.contains_member(&dave.device_id()),
            "Dave was not admitted"
        );
        assert_eq!(
            owner.stats.requests_rejected,
            rejected_before + 1,
            "the non-admin request was rejected"
        );
    }

    #[test]
    fn peer_descriptor_roundtrips_and_self_verifies() {
        let mut node = solo_node();
        node.publish_self_record(vec!["/ip4/1.2.3.4/tcp/9".into()], 5)
            .unwrap();
        let rec = node.self_record().unwrap().clone();
        assert!(rec.verify_self(), "a freshly-signed record self-verifies");
        assert_eq!(PeerDescriptor::decode(&rec.encode()).unwrap(), rec);
        // Tampering any signed field breaks the self-signature.
        let mut tampered = rec.clone();
        tampered.seq += 1;
        assert!(!tampered.verify_self());
    }

    #[test]
    fn pex_bundle_decoder_rejects_an_oversized_count() {
        // The receive-side count cap (MAX_PEX_ENTRIES) bounds verify work: a forged
        // header claiming more is rejected BEFORE the per-record decode/verify loop, so
        // a hostile member cannot force ~count Ed25519 verifications (the 3d-7 review's
        // blocking CPU-DoS).
        let mut over = Encoder::new();
        over.put_u32(MAX_PEX_ENTRIES as u32 + 1);
        assert!(matches!(
            decode_pex_bundle(&over.finish()),
            Err(SyncError::Malformed)
        ));
        let mut absurd = Encoder::new();
        absurd.put_u32(u32::MAX);
        assert!(matches!(
            decode_pex_bundle(&absurd.finish()),
            Err(SyncError::Malformed)
        ));
    }

    #[test]
    fn peer_descriptor_decode_rejects_an_overlong_address() {
        let long = "x".repeat(MAX_PEX_ADDR_LEN + 1);
        let mut e = Encoder::new();
        e.put_bytes(&[1u8; 32]).unwrap(); // device_pubkey
        e.put_bytes(&[2u8; 32]).unwrap(); // peer_id
        e.put_u64(1); // seq
        e.put_u32(1); // address count
        e.put_str(&long).unwrap();
        e.put_bytes(&[0u8; 64]).unwrap(); // signature
        assert!(matches!(
            PeerDescriptor::decode(&e.finish()),
            Err(SyncError::Malformed)
        ));
    }

    #[tokio::test]
    async fn ingest_peer_record_requires_a_member_and_a_valid_signature() {
        let (_hub, mut members, _ids) = build_members(2).await;
        let mut it = members.drain(..);
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        drop(it);

        bob.publish_self_record(vec!["/ip4/10.0.0.2/tcp/1".into()], 1)
            .unwrap();
        // Bob is a member of Alice's roster and his record self-verifies → accepted.
        assert!(alice.ingest_peer_record(bob.self_record().unwrap().clone()));

        // A record signed by a NON-member is dropped even though its self-signature
        // is valid (the signer is not in the roster).
        let mallory = MlsDevice::generate().unwrap();
        let addrs = vec!["/ip4/6.6.6.6/tcp/6".into()];
        let payload =
            peer_record_signing_payload(&mallory.public_key_bytes(), &[9u8; 32], &addrs, 1);
        let mal = PeerDescriptor {
            device_pubkey: mallory.public_key_bytes(),
            peer_id: [9u8; 32],
            addresses: addrs,
            seq: 1,
            signature: mallory.sign(&payload).unwrap(),
        };
        assert!(mal.verify_self(), "Mallory's self-signature is valid…");
        assert!(!alice.ingest_peer_record(mal), "…but she is not a member");

        // A member record with a tampered signature is dropped.
        let mut bad = bob.self_record().unwrap().clone();
        bad.signature[0] ^= 0xFF;
        assert!(!alice.ingest_peer_record(bad));

        // PEX ingestion never promotes anyone to the trusted catch-up pool.
        assert_eq!(alice.stats().member_peers, 0);
    }

    #[tokio::test]
    async fn pex_round_trip_learns_a_third_member_through_a_second() {
        // M1 (Alice, founder) asks M2 (Carol, last joiner with the full roster) for
        // its peer records, and learns M3 (Bob) — members supply each other with peers.
        let (_hub, members, ids) = build_members(3).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let mut carol = it.next().unwrap();
        let bob_id = ids[1];
        let carol_id = ids[2];

        bob.publish_self_record(vec!["/ip4/10.0.0.2/tcp/1".into()], 1)
            .unwrap();
        carol
            .publish_self_record(vec!["/ip4/10.0.0.3/tcp/1".into()], 1)
            .unwrap();
        // Carol already knows Bob's record (she joined after him, so Bob is in her
        // roster); seed it so she can relay it.
        assert!(carol.ingest_peer_record(bob.self_record().unwrap().clone()));

        let carol_peer = carol.local_peer();
        let (learned, _) = tokio::join!(alice.request_pex(carol_peer), carol.run_once());
        let learned = learned.unwrap();
        assert_eq!(learned, 2, "Alice learned Carol's own record + Bob's");
        assert!(
            alice.peer_record(&bob_id).is_some(),
            "Alice learned M3 (Bob) via PEX through M2 (Carol)"
        );
        assert!(alice.peer_record(&carol_id).is_some());
        // Discovery candidates only — no catch-up-source promotion.
        assert_eq!(alice.stats().member_peers, 0);
    }

    #[tokio::test]
    async fn a_blob_is_fetched_from_a_member_over_the_mesh() {
        let (_hub, members, _ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let alice_peer = alice.local_peer();

        // An address no member holds → Ok(false), not an error.
        let missing = Cid::of(b"no member holds this");
        let (got, _) = tokio::join!(bob.request_blob(alice_peer, &missing), alice.run_once());
        assert!(!got.unwrap(), "a blob no peer holds returns false");
        assert!(!bob.has_blob(&missing));

        // Alice holds a content-addressed blob (e.g. an avatar); Bob fetches it by address.
        let data = b"a content-addressed blob the gossip docs should not carry".to_vec();
        let cid = alice.put_blob(&data).unwrap();
        assert!(!bob.has_blob(&cid));
        let (fetched, _) = tokio::join!(bob.request_blob(alice_peer, &cid), alice.run_once());
        assert!(fetched.unwrap(), "Bob fetched the blob from a member");
        assert_eq!(bob.get_blob(&cid), Some(data));
        assert!(bob.has_blob(&cid), "and it is now held locally");
    }

    #[tokio::test]
    async fn blob_serving_is_rate_limited_per_requester() {
        // The injected clock is fixed in the test harness, so two serves land within
        // MIN_BLOB_INTERVAL_MS — the second is throttled to an empty reply.
        let (_hub, members, _ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let alice_peer = alice.local_peer();

        let cid1 = alice.put_blob(b"blob one").unwrap();
        let cid2 = alice.put_blob(b"blob two").unwrap();

        let (f1, _) = tokio::join!(bob.request_blob(alice_peer, &cid1), alice.run_once());
        assert!(f1.unwrap(), "first blob served");

        let (f2, _) = tokio::join!(bob.request_blob(alice_peer, &cid2), alice.run_once());
        assert!(
            !f2.unwrap(),
            "a second blob within the interval is rate-limited"
        );
        assert!(!bob.has_blob(&cid2));
    }

    #[tokio::test]
    async fn pex_to_a_non_member_is_rejected() {
        let mut alice = solo_node();
        alice
            .publish_self_record(vec!["/ip4/10.0.0.1/tcp/1".into()], 1)
            .unwrap();
        let gid = alice.group.group_id();
        // A non-member crafts a syntactically-valid authed PEX request; Alice's
        // membership gate refuses it (no records leaked) and counts the rejection.
        let mallory = MlsDevice::generate().unwrap();
        let inner: &[u8] = &[];
        let (ts, nonce, epoch) = (1_000u64, [0u8; 16], 0u64);
        let transcript = catchup_auth_transcript(
            &gid,
            KIND_PEX,
            inner,
            &mallory.public_key_bytes(),
            ts,
            &nonce,
            epoch,
        );
        let sig = mallory.sign(&transcript).unwrap();
        let body =
            encode_authed_request(inner, &mallory.public_key_bytes(), ts, &nonce, epoch, &sig);
        let before = alice.stats().requests_rejected;
        assert!(alice.serve_pex(PeerId::from_u64(99), &body).is_none());
        assert_eq!(alice.stats().requests_rejected, before + 1);
    }

    #[tokio::test]
    async fn pex_is_rate_limited_per_requester() {
        let (_hub, members, _ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        alice
            .publish_self_record(vec!["/ip4/10.0.0.1/tcp/1".into()], 1)
            .unwrap();
        let bob_peer = bob.local_peer();
        // Bob (a member) signs a PEX request; strip the leading kind byte for serve_pex.
        let (req, _auth) = bob.build_authed_request(KIND_PEX, &[]).unwrap();
        let body = &req[1..];
        let first = alice.serve_pex(bob_peer, body);
        assert!(
            matches!(&first, Some(b) if !b.is_empty()),
            "the first PEX is served"
        );
        let second = alice.serve_pex(bob_peer, body);
        assert_eq!(
            second,
            Some(Vec::new()),
            "a second PEX within the interval is rate-limited to an empty reply"
        );
    }
}
