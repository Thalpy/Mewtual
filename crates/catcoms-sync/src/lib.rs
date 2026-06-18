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
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt;

use automerge::{AutoCommit, AutomergeError};
use bytes::Bytes;
use catcoms_crypto::{verify_with_public_bytes, DeviceId};
use catcoms_mls::{serialize_key_package, InviteLedger, InviteToken, MlsDevice, ServerGroup};
use catcoms_replication::{EncryptedDoc, SealedOp};
use catcoms_rt::{Clock, CryptoRngCore, MeshTransport, PeerId, ProtocolId, Topic, TransportEvent};
use catcoms_wire::{Decoder, DocType, Encoder};
use thiserror::Error;
use zeroize::Zeroizing;

/// Request/response protocol (one RR protocol; the first payload byte selects the
/// request kind so it works over the single-protocol mesh node).
const RR_PROTOCOL: &str = "/catcoms/rr/1";
/// Request kind: catch-up (document history) request.
const KIND_CATCHUP: u8 = 0;
/// Request kind: join (admission) request.
const KIND_JOIN: u8 = 1;
/// Request kind: membership commit catch-up (missed-commit recovery, 6d-1b).
const KIND_COMMIT_CATCHUP: u8 = 2;
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
        }
    }
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
    /// Gauge: past-epoch channel keys currently retained.
    pub past_keys_retained: usize,
    /// Gauge: membership commits currently buffered out of order.
    pub pending_commits: usize,
    /// Gauge: membership commits currently retained to serve catch-up.
    pub commit_log_len: usize,
    /// Gauge: peers currently known as catch-up sources.
    pub known_peers: usize,
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
fn join_transcript(group_id: &[u8], invite_nonce: &[u8; 16], welcome: &[u8]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(JOIN_RESP_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(invite_nonce).expect("16 fits");
    e.put_bytes(welcome).expect("welcome fits");
    e.finish()
}

/// The transcript a requester signs to prove it is a current member when asking
/// for catch-up, binding the proof to the group, the request kind + body, the
/// requester's own key, and a freshness timestamp.
fn catchup_auth_transcript(
    group_id: &[u8],
    kind: u8,
    inner: &[u8],
    requester_pubkey: &[u8],
    timestamp_ms: u64,
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(CATCHUP_AUTH_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_u16(kind as u16);
    e.put_bytes(inner).expect("inner fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(timestamp_ms);
    e.finish()
}

/// Frame an authenticated catch-up request body: `inner ‖ pubkey ‖ ts ‖ sig`.
fn encode_authed_request(
    inner: &[u8],
    requester_pubkey: &[u8],
    timestamp_ms: u64,
    signature: &[u8; 64],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(inner).expect("inner fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(timestamp_ms);
    e.put_bytes(signature).expect("64 fits");
    e.finish()
}

/// A parsed authenticated request: `(inner body, requester pubkey, timestamp, signature)`.
type AuthedRequest = (Vec<u8>, Vec<u8>, u64, [u8; 64]);

/// Parse an authenticated catch-up request body into `(inner, pubkey, ts, sig)`.
fn decode_authed_request(bytes: &[u8]) -> Result<AuthedRequest, SyncError> {
    let mut d = Decoder::new(bytes);
    let inner = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let pubkey = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let timestamp_ms = d.get_u64().map_err(|_| SyncError::Malformed)?;
    let signature: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((inner, pubkey, timestamp_ms, signature))
}

fn encode_join_resp(welcome: &[u8], signature: &[u8; 64]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(welcome).expect("welcome fits");
    e.put_bytes(signature).expect("64 fits");
    e.finish()
}

fn decode_join_resp(bytes: &[u8]) -> Result<(Vec<u8>, [u8; 64]), SyncError> {
    let mut d = Decoder::new(bytes);
    let welcome = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let signature: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((welcome, signature))
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
    /// The requested document is not open here.
    #[error("document not open")]
    NoSuchDoc,
    /// A join request was rejected by the inviter.
    #[error("join request rejected")]
    JoinRejected,
    /// A sync message was malformed.
    #[error("malformed sync message")]
    Malformed,
}

/// The blinded gossip topic for a document: `BLAKE3(label ‖ group_id ‖ type ‖ id)`.
/// Only members (who know the random group id) can compute it.
fn channel_topic(group_id: &[u8], doc_type: DocType, doc_id: u128) -> Topic {
    let mut h = blake3::Hasher::new();
    h.update(b"catcoms/topic/v1");
    h.update(group_id);
    h.update(&doc_type.tag().to_be_bytes());
    h.update(&doc_id.to_be_bytes());
    Topic::new(h.finalize().as_bytes().to_vec())
}

/// The per-group **control** topic carrying membership commits. Stable per group
/// (independent of epoch), so it survives the epoch bumps a membership change
/// causes.
fn control_topic(group_id: &[u8]) -> Topic {
    let mut h = blake3::Hasher::new();
    h.update(b"catcoms/control/v1");
    h.update(group_id);
    Topic::new(h.finalize().as_bytes().to_vec())
}

/// A membership commit fanned out on the control topic so existing members apply
/// it and advance to the same epoch. `commit_epoch` is the epoch the commit was
/// built at (it advances the group to `commit_epoch + 1`) — the linearization key
/// for ordered replay during recovery.
#[derive(Debug, Clone)]
struct CommitRecord {
    group_id: Vec<u8>,
    commit_epoch: u64,
    committer_device: [u8; 32],
    mls_commit: Vec<u8>,
}

impl CommitRecord {
    fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(&self.group_id).expect("group id fits");
        e.put_u64(self.commit_epoch);
        e.put_bytes(&self.committer_device).expect("32 fits");
        e.put_bytes(&self.mls_commit).expect("commit fits");
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
        d.finish().map_err(|_| SyncError::Malformed)?;
        Ok(Self {
            group_id,
            commit_epoch,
            committer_device,
            mls_commit,
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
pub struct ChannelSync<T: MeshTransport, R: CryptoRngCore> {
    transport: T,
    group: ServerGroup,
    device: MlsDevice,
    rng: R,
    clock: Box<dyn Clock + Send>,
    ledger: InviteLedger,
    docs: HashMap<(DocType, u128), EncryptedDoc>,
    control_topic: Topic,
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
    /// Recently-seen peers (from gossip/requests/connections), used as catch-up
    /// sources — there is no `DeviceId → PeerId` directory yet.
    known_peers: VecDeque<PeerId>,
    /// Recovery work to perform on the next async drain.
    catchup_queue: Vec<CatchupTask>,
    /// Peers that recently answered a commit catch-up without filling the gap;
    /// skipped when choosing the next catch-up source so one bad/stale peer can't
    /// dead-end recovery. Cleared once a catch-up makes progress.
    failed_catchup_peers: VecDeque<PeerId>,
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
        rng: R,
        clock: Box<dyn Clock + Send>,
    ) -> Self {
        let control_topic = control_topic(&group.group_id());
        Self {
            transport,
            group,
            device,
            rng,
            clock,
            ledger: InviteLedger::new(),
            docs: HashMap::new(),
            control_topic,
            config: SyncConfig::default(),
            outbox: Vec::new(),
            commit_log: VecDeque::new(),
            pending_commits: BTreeMap::new(),
            past_keys: BTreeMap::new(),
            known_peers: VecDeque::new(),
            catchup_queue: Vec::new(),
            failed_catchup_peers: VecDeque::new(),
            stats: SyncStats::default(),
        }
    }

    /// Override the recovery/key-window bounds (defaults to [`SyncConfig::default`]).
    /// Useful for tests and for tuning a constrained device.
    pub fn set_config(&mut self, config: SyncConfig) {
        self.config = config;
    }

    /// Subscribe to this group's control topic so the member receives membership
    /// commits. Call once after construction (and after joining).
    pub async fn subscribe_control(&mut self) -> Result<(), SyncError> {
        self.transport.subscribe(self.control_topic.clone()).await?;
        Ok(())
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

    /// Open a document: create it locally (if absent) and subscribe to its topic.
    pub async fn open_channel(&mut self, doc_type: DocType, doc_id: u128) -> Result<(), SyncError> {
        let key = (doc_type, doc_id);
        let actor = self.device.device_id();
        self.docs
            .entry(key)
            .or_insert_with(|| EncryptedDoc::new(doc_type, doc_id, &actor));
        let topic = channel_topic(&self.group.group_id(), doc_type, doc_id);
        tracing::info!(
            ?doc_type,
            doc_id,
            epoch = self.group.epoch(),
            "open channel"
        );
        self.transport.subscribe(topic).await?;
        Ok(())
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
        let topic = channel_topic(&self.group.group_id(), doc_type, doc_id);
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
        // Flush any queued membership-commit broadcasts (retries previous failures).
        self.drain_outbox().await;
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
                if topic == self.control_topic {
                    self.on_control(&data);
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
                let response = self.handle_request(&data);
                // Broadcast any membership commit produced by serving the request
                // BEFORE telling the joiner it succeeded, so a crash leaves the
                // joiner to retry rather than the group silently missing the Add.
                self.drain_outbox().await;
                responder.respond(Bytes::from(response));
                Ok(true)
            }
            Some(TransportEvent::PeerConnected(peer)) => {
                self.remember_peer(peer);
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

    /// Pick the most-recently-seen peer that has not just failed to fill a gap.
    fn pick_catchup_peer(&self) -> Option<PeerId> {
        self.known_peers
            .iter()
            .rev()
            .find(|p| !self.failed_catchup_peers.contains(p))
            .copied()
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

    /// Build an authenticated catch-up request: `[kind] ‖ inner ‖ pubkey ‖ ts ‖ sig`,
    /// where the signature is this member's proof it is currently in the group.
    fn build_authed_request(&self, kind: u8, inner: &[u8]) -> Result<Vec<u8>, SyncError> {
        let pubkey = self.device.public_key_bytes();
        let ts = self.clock.now_ms();
        let transcript = catchup_auth_transcript(&self.group.group_id(), kind, inner, &pubkey, ts);
        let signature = self.device.sign(&transcript)?;
        let mut out = vec![kind];
        out.extend_from_slice(&encode_authed_request(inner, &pubkey, ts, &signature));
        Ok(out)
    }

    /// Verify an inbound authenticated catch-up request and return its inner body
    /// iff the requester proved current group membership with a fresh signature.
    /// Counts a rejection in [`SyncStats`]. `kind` is the matched request kind.
    fn authenticate_request(&mut self, kind: u8, data: &[u8]) -> Option<Vec<u8>> {
        let (inner, pubkey, ts, signature) = match decode_authed_request(data) {
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
        let transcript = catchup_auth_transcript(&self.group.group_id(), kind, &inner, &pubkey, ts);
        if !verify_with_public_bytes(&pubkey, &transcript, &signature) {
            tracing::warn!("catch-up request signature invalid; refused");
            self.stats.requests_rejected += 1;
            return None;
        }
        Some(inner)
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

    /// Append a commit to the retained log (so this node can serve catch-up),
    /// bounded by `max_commit_log`.
    fn record_commit(&mut self, record: CommitRecord) {
        self.commit_log.push_back(record);
        while self.commit_log.len() > self.config.max_commit_log {
            self.commit_log.pop_front();
        }
    }

    /// Apply a commit that is exactly the next one (`commit_epoch == current`),
    /// capturing the soon-to-be-superseded epoch's keys first. Returns whether it
    /// applied.
    fn apply_commit_in_order(&mut self, record: &CommitRecord) -> bool {
        debug_assert_eq!(record.commit_epoch, self.group.epoch());
        // Single-committer policy, enforced on the inbound path too: the commit's
        // claimed committer must be the epoch's designated committer (lowest leaf
        // index). openmls still independently authenticates the commit, but this
        // makes `committer_device` meaningful and rejects a mislabeled/forked one
        // before we spend work on it. (Full per-commit committer signatures are a
        // 6d-2 concern.)
        let expected = self.group.designated_committer();
        if expected != Some(DeviceId::from_bytes(record.committer_device)) {
            tracing::warn!(
                commit_epoch = record.commit_epoch,
                "membership commit not from the designated committer; rejected"
            );
            return false;
        }
        self.snapshot_epoch_keys();
        match self
            .group
            .process_incoming(&self.device, &record.mls_commit)
        {
            Ok(_) => {
                self.evict_past_keys();
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
    }

    /// Apply an inbound membership commit from the control topic. A commit that is
    /// exactly the next one is applied immediately (then any buffered successors
    /// drain in order); one that is ahead of us is buffered and triggers
    /// commit-catch-up; an already-applied one is ignored.
    fn on_control(&mut self, data: &[u8]) {
        let record = match CommitRecord::decode(data) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "dropping malformed control message");
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
                if self.apply_commit_in_order(&record) {
                    self.drain_pending_commits();
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
        let req = self.build_authed_request(KIND_CATCHUP, &encode_catchup_req(doc_type, doc_id))?;
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
        let req =
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
        let records = decode_commit_bundle(&resp)?;
        let group_id = self.group.group_id();
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
    /// empty response uniformly signals "nothing / rejected").
    fn handle_request(&mut self, data: &[u8]) -> Vec<u8> {
        if data.len() > MAX_CONTROL_REQUEST {
            tracing::warn!(bytes = data.len(), "oversized control request dropped");
            return Vec::new();
        }
        match data.split_first() {
            Some((&KIND_CATCHUP, rest)) => self.serve_catchup(rest).unwrap_or_default(),
            Some((&KIND_JOIN, rest)) => self.serve_join(rest).unwrap_or_default(),
            Some((&KIND_COMMIT_CATCHUP, rest)) => {
                self.serve_commit_catchup(rest).unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }

    /// Serve a document's history — but only to a requester that proved current
    /// group membership (the bundle, though sealed, still carries member-only
    /// framing/metadata, so it is members-only).
    fn serve_catchup(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let inner = self.authenticate_request(KIND_CATCHUP, data)?;
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
        let inner = self.authenticate_request(KIND_COMMIT_CATCHUP, data)?;
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
        Some(encode_commit_bundle(&records))
    }

    /// Admit a joiner from a join request. Cheap, KeyPackage-independent checks run
    /// first (so junk requests never pay for KeyPackage validation); only the
    /// invite's named inviter admits over the network (so the joiner can
    /// authenticate the response via the invite's public key). On success the
    /// nonce is consumed and a Welcome **signed by this (inviter) device** is
    /// returned. Returns `None` (empty response) on any failure.
    fn serve_join(&mut self, data: &[u8]) -> Option<Vec<u8>> {
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
        // Single-committer model: only the designated committer (lowest leaf
        // index) may produce commits, so concurrent admits cannot fork the epoch
        // chain. (Routing admissions to the committer is a later block.)
        if !self.group.is_designated_committer(&self.device) {
            tracing::warn!("not the designated committer; cannot admit in this block");
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

        // --- expensive: validate the KeyPackage, then admit ---
        let key_package = self.device.parse_key_package(&kp_bytes).ok()?;
        // Capture our current-epoch channel keys before the Add advances the
        // epoch, so an op a peer sealed at this epoch still opens for us.
        self.snapshot_epoch_keys();
        let outcome = self
            .group
            .add_member_via_invite(&self.device, key_package, &invite, &mut self.ledger, now)
            .ok()?;
        self.evict_past_keys();

        // Retain the Add commit: queue it for fan-out on the control topic, and
        // keep it in the commit log so we can later serve commit-catch-up to any
        // member that missed the broadcast.
        let record = CommitRecord {
            group_id: self.group.group_id(),
            commit_epoch: outcome.commit_epoch,
            committer_device: *self.device.device_id().as_bytes(),
            mls_commit: outcome.commit,
        };
        self.record_commit(record.clone());
        self.outbox
            .push((self.control_topic.clone(), record.encode()));

        // Sign the Welcome so the joiner can authenticate it came from the inviter.
        let transcript = join_transcript(&invite.group_id, &invite.invite_nonce, &outcome.welcome);
        let signature = self.device.sign(&transcript).ok()?;
        tracing::info!(epoch = self.group.epoch(), "admitted a member via invite");
        Some(encode_join_resp(&outcome.welcome, &signature))
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
) -> Result<ServerGroup, SyncError> {
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
    if resp.is_empty() {
        return Err(SyncError::JoinRejected);
    }
    let (welcome, signature) = decode_join_resp(&resp)?;

    // The Welcome must be signed by the inviter named in the invite.
    let transcript = join_transcript(&invite.group_id, &invite.invite_nonce, &welcome);
    if !invite.verify_inviter_signature(&transcript, &signature) {
        tracing::warn!("join response was not signed by the invite's inviter");
        return Err(SyncError::JoinRejected);
    }

    let group = ServerGroup::join(device, &welcome)?;
    // Defense in depth: we must have landed in the group the invite named.
    if group.group_id() != invite.group_id {
        tracing::warn!("joined group id does not match the invite");
        return Err(SyncError::JoinRejected);
    }
    tracing::info!(epoch = group.epoch(), "joined server via invite");
    Ok(group)
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
    fn commit_catchup_request_roundtrips_through_codec() {
        let bytes = encode_commit_catchup_req(42);
        assert_eq!(decode_commit_catchup_req(&bytes).unwrap(), 42);
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
            },
            CommitRecord {
                group_id: b"gid".to_vec(),
                commit_epoch: 2,
                committer_device: [4u8; 32],
                mls_commit: vec![1, 2, 3, 4],
            },
        ];
        let decoded = decode_commit_bundle(&encode_commit_bundle(&records)).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].commit_epoch, 1);
        assert_eq!(decoded[1].mls_commit, vec![1, 2, 3, 4]);
    }

    /// A genuine membership commit applied on the inbound path is accepted; the
    /// same commit relabeled with a different `committer_device` is rejected (the
    /// single-committer policy is enforced when applying, not only when admitting).
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
        let mut bsy = ChannelSync::new(
            bob_net,
            bob_joined.unwrap(),
            bob,
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
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
        bsy.on_control(&forged.encode());
        assert_eq!(bsy.epoch(), 1, "forged-committer commit must be rejected");
        assert_eq!(bsy.stats().commits_applied, 0);

        // The genuine commit (committer = Alice) applies.
        bsy.on_control(&genuine.encode());
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
}
