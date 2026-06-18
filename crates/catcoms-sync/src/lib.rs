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
//!   member's signed-op log re-sealed under the current epoch.
//!
//! Deferred to later mesh sub-blocks: rotating the blinded topic on member
//! removal (here it is stable per group+document), and proposal/commit
//! linearization of membership changes.

use std::collections::HashMap;
use std::fmt;

use automerge::{AutoCommit, AutomergeError};
use bytes::Bytes;
use catcoms_mls::{serialize_key_package, InviteLedger, InviteToken, MlsDevice, ServerGroup};
use catcoms_replication::{EncryptedDoc, SealedOp};
use catcoms_rt::{Clock, CryptoRngCore, MeshTransport, PeerId, ProtocolId, Topic, TransportEvent};
use catcoms_wire::{Decoder, DocType, Encoder};
use thiserror::Error;

/// Request/response protocol (one RR protocol; the first payload byte selects the
/// request kind so it works over the single-protocol mesh node).
const RR_PROTOCOL: &str = "/catcoms/rr/1";
/// Request kind: catch-up (history) request.
const KIND_CATCHUP: u8 = 0;
/// Request kind: join (admission) request.
const KIND_JOIN: u8 = 1;
/// Control requests (join/catch-up) are small; reject anything larger up front.
const MAX_CONTROL_REQUEST: usize = 64 * 1024;
/// Domain separator for the admitter's signature over a join response.
const JOIN_RESP_DOMAIN: &str = "catcoms/join-resp/v1";

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
/// built at (it advances the group to `commit_epoch + 1`) — the linearization key.
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

fn encode_bundle(ops: &[SealedOp]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u32(ops.len() as u32);
    for op in ops {
        e.put_bytes(&op.encode()).expect("op fits");
    }
    e.finish()
}

fn decode_bundle(bytes: &[u8]) -> Result<Vec<SealedOp>, SyncError> {
    let mut d = Decoder::new(bytes);
    let count = d.get_u32().map_err(|_| SyncError::Malformed)?;
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
    /// Membership commits queued for the control topic (drained in async run_once).
    outbox: Vec<(Topic, Vec<u8>)>,
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
            outbox: Vec::new(),
        }
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
        tracing::info!(?doc_type, doc_id, "open channel");
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
        tracing::debug!(?doc_type, doc_id, bytes = bytes.len(), "post op");
        self.transport.publish(topic, Bytes::from(bytes)).await?;
        Ok(())
    }

    /// Process one inbound transport event (gossiped op, membership commit, or a
    /// catch-up / join request). Returns `false` when the transport has closed.
    pub async fn run_once(&mut self) -> Result<bool, SyncError> {
        // Flush any queued membership-commit broadcasts (retries previous failures).
        self.drain_outbox().await;

        let event = self.transport.next_event().await;
        match event {
            None => Ok(false),
            Some(TransportEvent::Gossip { topic, data, .. }) => {
                if topic == self.control_topic {
                    self.on_control(&data);
                } else {
                    self.on_gossip(&data);
                }
                Ok(true)
            }
            Some(TransportEvent::Request {
                data, responder, ..
            }) => {
                let response = self.handle_request(&data);
                // Broadcast any membership commit produced by serving the request
                // BEFORE telling the joiner it succeeded, so a crash leaves the
                // joiner to retry rather than the group silently missing the Add.
                self.drain_outbox().await;
                responder.respond(Bytes::from(response));
                Ok(true)
            }
            Some(_) => Ok(true),
        }
    }

    /// Publish all queued control-topic broadcasts; re-queue any that fail.
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
    }

    /// Apply an inbound membership commit from the control topic. In the
    /// single-committer model, commits arrive in epoch order under reliable
    /// delivery; an out-of-order commit is logged and dropped (the buffering +
    /// commit-log catch-up recovery net is a later block).
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
        if record.commit_epoch != current {
            tracing::warn!(
                commit_epoch = record.commit_epoch,
                current,
                "out-of-order membership commit dropped (recovery net is a later block)"
            );
            return;
        }
        match self
            .group
            .process_incoming(&self.device, &record.mls_commit)
        {
            Ok(_) => tracing::info!(epoch = self.group.epoch(), "applied membership commit"),
            Err(e) => tracing::warn!(error = %e, "failed to apply membership commit"),
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
        let mut req = vec![KIND_CATCHUP];
        req.extend_from_slice(&encode_catchup_req(doc_type, doc_id));
        tracing::debug!(?doc_type, doc_id, "request catch-up");
        let resp = self
            .transport
            .request(peer, ProtocolId(RR_PROTOCOL), Bytes::from(req))
            .await?;
        if resp.is_empty() {
            return Ok(0); // peer had nothing for this document
        }
        let bundle = decode_bundle(&resp)?;

        let key = (doc_type, doc_id);
        let actor = self.device.device_id();
        self.docs
            .entry(key)
            .or_insert_with(|| EncryptedDoc::new(doc_type, doc_id, &actor));
        let doc = self.docs.get_mut(&key).expect("just inserted");
        let applied = doc.import_catchup(&bundle, &self.group, &self.device)?;
        tracing::debug!(applied, "applied catch-up");
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
        if let Some(doc) = self.docs.get_mut(&key) {
            match doc.ingest(&sealed, &self.group, &self.device) {
                Ok(true) => tracing::trace!(?key, "ingested op"),
                Ok(false) => tracing::trace!(?key, "duplicate op ignored"),
                Err(e) => tracing::warn!(error = %e, "rejected inbound op"),
            }
        } else {
            tracing::trace!(?key, "gossip for unopened document, ignoring");
        }
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
            _ => Vec::new(),
        }
    }

    fn serve_catchup(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let (doc_type, doc_id) = decode_catchup_req(data).ok()?;
        let doc = self.docs.get(&(doc_type, doc_id))?;
        match doc.export_catchup(&self.group, &self.device, &mut self.rng) {
            Ok(bundle) => {
                tracing::debug!(?doc_type, doc_id, ops = bundle.len(), "serving catch-up");
                Some(encode_bundle(&bundle))
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to build catch-up bundle");
                None
            }
        }
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
        let outcome = self
            .group
            .add_member_via_invite(&self.device, key_package, &invite, &mut self.ledger, now)
            .ok()?;

        // Queue the Add commit for fan-out to existing members on the control
        // topic (run_once publishes it; this member already merged it locally).
        let record = CommitRecord {
            group_id: self.group.group_id(),
            commit_epoch: outcome.commit_epoch,
            committer_device: *self.device.device_id().as_bytes(),
            mls_commit: outcome.commit,
        };
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
            .field("open_docs", &self.docs.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catchup_request_roundtrips_through_codec() {
        let bytes = encode_catchup_req(DocType::Wiki, 99);
        assert_eq!(decode_catchup_req(&bytes).unwrap(), (DocType::Wiki, 99));
    }

    #[test]
    fn malicious_responder_cannot_divert_joiner_into_its_own_group() {
        use catcoms_mls::{MlsDevice, ServerGroup};

        // Legit inviter Alice issues an invite to her group A.
        let alice = MlsDevice::generate().unwrap();
        let alice_group = ServerGroup::create(&alice).unwrap();
        let invite = alice_group
            .mint_invite(&alice, [7u8; 16], 10_000, vec![])
            .unwrap();

        // Mallory controls her own group M and intercepts Bob's join.
        let mallory = MlsDevice::generate().unwrap();
        let mut mallory_group = ServerGroup::create(&mallory).unwrap();

        // Bob mints a KeyPackage bound to the invite (group A); its init key is not
        // a secret, so Mallory can add it to HER group and craft a valid Welcome.
        let bob = MlsDevice::generate().unwrap();
        let bob_kp = bob
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let welcome = mallory_group.add_member(&mallory, bob_kp).unwrap().welcome;
        let transcript = join_transcript(&invite.group_id, &invite.invite_nonce, &welcome);
        let mallory_sig = mallory.sign(&transcript).unwrap();

        // Defense 1: the response signature does NOT verify against the invite's
        // inviter (Mallory is not Alice).
        assert!(!invite.verify_inviter_signature(&transcript, &mallory_sig));

        // Defense 2 (even if the signature somehow passed): the group Bob would
        // land in is not the one the invite names.
        let diverted = ServerGroup::join(&bob, &welcome).unwrap();
        assert_ne!(diverted.group_id(), invite.group_id);
    }

    #[test]
    fn topic_is_stable_and_distinguishes_documents() {
        let g = b"group-id-bytes";
        assert_eq!(
            channel_topic(g, DocType::Channel, 1),
            channel_topic(g, DocType::Channel, 1)
        );
        assert_ne!(
            channel_topic(g, DocType::Channel, 1),
            channel_topic(g, DocType::Channel, 2)
        );
        assert_ne!(
            channel_topic(g, DocType::Channel, 1),
            channel_topic(b"other-group", DocType::Channel, 1)
        );
    }
}
