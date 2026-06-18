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
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_replication::{EncryptedDoc, SealedOp};
use catcoms_rt::{CryptoRngCore, MeshTransport, ProtocolId, Topic, TransportEvent};
use catcoms_wire::{Decoder, DocType, Encoder};
use thiserror::Error;

/// Request/response protocol for catch-up (matches the mesh node's RR protocol).
const CATCHUP_PROTOCOL: &str = "/catcoms/rr/1";

/// Errors from channel synchronization.
#[derive(Debug, Error)]
pub enum SyncError {
    /// A transport-level error.
    #[error(transparent)]
    Transport(#[from] catcoms_rt::TransportError),
    /// A replication-layer error.
    #[error(transparent)]
    Repl(#[from] catcoms_replication::ReplError),
    /// The requested document is not open here.
    #[error("document not open")]
    NoSuchDoc,
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

/// Synchronizes a server's documents over a [`MeshTransport`]. Generic over the
/// injected RNG `R` (e.g. `OsCryptoRng` in production, a seeded CSPRNG in tests).
pub struct ChannelSync<T: MeshTransport, R: CryptoRngCore> {
    transport: T,
    group: ServerGroup,
    device: MlsDevice,
    rng: R,
    docs: HashMap<(DocType, u128), EncryptedDoc>,
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Build a synchronizer over `transport` for this member's `group`/`device`.
    pub fn new(transport: T, group: ServerGroup, device: MlsDevice, rng: R) -> Self {
        Self {
            transport,
            group,
            device,
            rng,
            docs: HashMap::new(),
        }
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

    /// Process one inbound transport event (gossiped op or catch-up request).
    /// Returns `false` when the transport has closed.
    pub async fn run_once(&mut self) -> Result<bool, SyncError> {
        let event = self.transport.next_event().await;
        match event {
            None => Ok(false),
            Some(TransportEvent::Gossip { data, .. }) => {
                self.on_gossip(&data);
                Ok(true)
            }
            Some(TransportEvent::Request {
                data, responder, ..
            }) => {
                if let Some(bytes) = self.serve_catchup(&data) {
                    responder.respond(Bytes::from(bytes));
                }
                Ok(true)
            }
            Some(_) => Ok(true),
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
        let req = encode_catchup_req(doc_type, doc_id);
        tracing::debug!(?doc_type, doc_id, "request catch-up");
        let resp = self
            .transport
            .request(peer, ProtocolId(CATCHUP_PROTOCOL), Bytes::from(req))
            .await?;
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
