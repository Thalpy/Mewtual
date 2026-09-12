//! Deterministic P1 checkpoint changes and receipt-bound installation.
//!
//! A seed is not a user operation: it has no device signature and does not spend the open
//! epoch's content budget. Only a verified owner's receipt authorizes its exact change hash.
//! Consumers supply their typed projection writer/validator; this module fixes every Automerge
//! change field and refuses compressed/document chunks before invoking Automerge's parser.

use automerge::transaction::CommitOptions;
use automerge::{ActorId, AutoCommit, Change, ReadDoc, ROOT};
use catcoms_wire::{Decoder, DocType, Encoder};
use sha2::{Digest, Sha256};

use crate::epoch::hash_parts;
use crate::{epoch_id, LogicalDocument, ReplError, VerifiedReceipt};

/// Encoded raw seed limit, independent of the surrounding transport/padding allowance.
pub const MAX_CHECKPOINT_BYTES: usize = 2 * 1024 * 1024;

/// The checkpoint origin retained in the vault snapshot. It is not a network authorization:
/// network installation requires [`VerifiedReceipt`], while restore trusts the sealed vault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointOrigin {
    document: LogicalDocument,
    epoch: u64,
    close_hash: [u8; 32],
    seed_hash: [u8; 32],
}

impl CheckpointOrigin {
    /// Stable logical scope, including the server.
    pub fn document(&self) -> &LogicalDocument {
        &self.document
    }
    /// Epoch opened by the receipt.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    /// Exact seed excluded from subsequent close content accounting.
    pub fn seed_hash(&self) -> [u8; 32] {
        self.seed_hash
    }
    /// Concrete successor id; another close always derives another document.
    pub fn doc_id(&self) -> u128 {
        epoch_id(
            self.document.doc_type,
            &self.document.logical_key,
            self.epoch,
            &self.close_hash,
        )
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, ReplError> {
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_bytes(&self.document.server_id)
            .map_err(|_| ReplError::Malformed)?;
        e.put_u16(self.document.doc_type.tag());
        e.put_bytes(&self.document.logical_key)
            .map_err(|_| ReplError::Malformed)?;
        e.put_u64(self.epoch);
        e.put_bytes(&self.close_hash)
            .map_err(|_| ReplError::Malformed)?;
        e.put_bytes(&self.seed_hash)
            .map_err(|_| ReplError::Malformed)?;
        Ok(e.finish())
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > 1024 {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        let server = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let tag = d.get_u16().map_err(|_| ReplError::Malformed)?;
        let kind = DocType::from_tag(tag).ok_or(ReplError::Malformed)?;
        let key = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let epoch = d.get_u64().map_err(|_| ReplError::Malformed)?;
        let close_hash = d
            .get_bytes()
            .map_err(|_| ReplError::Malformed)?
            .try_into()
            .map_err(|_| ReplError::Malformed)?;
        let seed_hash = d
            .get_bytes()
            .map_err(|_| ReplError::Malformed)?
            .try_into()
            .map_err(|_| ReplError::Malformed)?;
        d.finish().map_err(|_| ReplError::Malformed)?;
        if epoch == 0 {
            return Err(ReplError::EpochScope);
        }
        let result = Self {
            document: LogicalDocument::new(server, kind, key)?,
            epoch,
            close_hash,
            seed_hash,
        };
        if result.encode()? != bytes {
            return Err(ReplError::Malformed);
        }
        Ok(result)
    }
}

/// A locally constructed seed candidate. Building it does not authorize shared editing.
#[derive(Clone, Debug)]
pub struct CheckpointSeed {
    origin: CheckpointOrigin,
    bytes: Vec<u8>,
}

/// Seed bytes whose exact hash, scope and shape have passed receipt and schema verification.
/// Private fields prevent a caller from replacing the bytes after verification.
#[derive(Clone, Debug)]
pub struct VerifiedCheckpoint {
    seed: CheckpointSeed,
}

impl VerifiedCheckpoint {
    /// Checkpoint identity to persist with its document.
    pub fn origin(&self) -> &CheckpointOrigin {
        &self.seed.origin
    }
    /// Raw change to serve by the receipt's expected hash, re-sealed by the transport.
    pub fn bytes(&self) -> &[u8] {
        &self.seed.bytes
    }
}

impl CheckpointSeed {
    /// Build exactly one canonical change. The typed writer must emit only live projection and
    /// bounded conflict data, in its documented canonical order. It must not copy a source DAG,
    /// markers, tombstones or device actors. No bytes are published or persisted here.
    pub fn build<F>(
        document: &LogicalDocument,
        checkpoint_epoch: u64,
        close_hash: [u8; 32],
        write_projection: F,
    ) -> Result<Self, ReplError>
    where
        F: FnOnce(&mut AutoCommit) -> Result<(), ReplError>,
    {
        LogicalDocument::new(
            document.server_id.clone(),
            document.doc_type,
            document.logical_key.clone(),
        )?;
        if checkpoint_epoch == 0 {
            return Err(ReplError::EpochScope);
        }
        let id = epoch_id(
            document.doc_type,
            &document.logical_key,
            checkpoint_epoch,
            &close_hash,
        );
        let actor = seed_actor(id);
        let mut doc = AutoCommit::new().with_actor(ActorId::from(actor.to_vec()));
        write_projection(&mut doc)?;
        doc.commit_with(CommitOptions::default().with_time(0).with_message(""));
        let change = doc.get_last_local_change().ok_or(ReplError::NoChange)?;
        let bytes = change.raw_bytes().to_vec();
        let origin = CheckpointOrigin {
            document: document.clone(),
            epoch: checkpoint_epoch,
            close_hash,
            seed_hash: change.hash().0,
        };
        validate_change(&origin, &bytes)?;
        reject_markers(&doc)?;
        Ok(Self { origin, bytes })
    }

    /// Complete encoded seed, used by the per-edit preflight and eventual publication.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Hash the owner must include in its persist-before-publish receipt decision.
    pub fn change_hash(&self) -> [u8; 32] {
        self.origin.seed_hash
    }
    /// Concrete successor identity.
    pub fn origin(&self) -> &CheckpointOrigin {
        &self.origin
    }

    /// Authenticate the expected raw bytes before parsing, then enforce the exact one-root
    /// shape and the consumer's schema. A forwarded receipt hint cannot call this entry point
    /// until owner/tenure verification has produced its opaque capability.
    pub fn verify<F>(
        receipt: &VerifiedReceipt,
        bytes: &[u8],
        validate_projection: F,
    ) -> Result<VerifiedCheckpoint, ReplError>
    where
        F: FnOnce(&LogicalDocument, u64, &AutoCommit) -> Result<(), ReplError>,
    {
        let origin = CheckpointOrigin {
            document: receipt.document().clone(),
            epoch: receipt
                .closed_epoch()
                .checked_add(1)
                .ok_or(ReplError::EpochBound)?,
            close_hash: receipt.close_record_hash(),
            seed_hash: receipt.seed_change_hash(),
        };
        let change = validate_change(&origin, bytes)?;
        let mut doc =
            AutoCommit::new().with_actor(ActorId::from(seed_actor(origin.doc_id()).to_vec()));
        doc.apply_changes([change]).map_err(am_error)?;
        reject_markers(&doc)?;
        validate_projection(&origin.document, origin.epoch, &doc)?;
        Ok(VerifiedCheckpoint {
            seed: Self {
                origin,
                bytes: bytes.to_vec(),
            },
        })
    }
}

/// This derivation's 16-byte document id is a raw byte string, not a u64 integer part.
pub fn seed_actor(doc_id: u128) -> [u8; 32] {
    hash_parts("catcoms-seed-actor:v1", &[&doc_id.to_be_bytes()])
}

pub(crate) fn validate_change(
    origin: &CheckpointOrigin,
    bytes: &[u8],
) -> Result<Change, ReplError> {
    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err(ReplError::EpochBound);
    }
    // Automerge 0.10 raw change header: magic (4), checksum (4), kind (1), canonical unsigned
    // LEB128 length, payload. Kind 2 invokes decompression; P1 only accepts raw kind 1. Its full
    // change hash is SHA-256(kind || canonical length || payload), pinned by a raw-byte vector.
    if bytes.len() < 10 || bytes[..4] != [0x85, 0x6f, 0x4a, 0x83] || bytes[8] != 1 {
        return Err(ReplError::Malformed);
    }
    let hash: [u8; 32] = Sha256::digest(&bytes[8..]).into();
    if hash != origin.seed_hash || bytes[4..8] != hash[..4] {
        return Err(ReplError::BadSignature);
    }
    crate::bounded_change::check(bytes, crate::bounded_change::MAX_ACTIONS)?;
    let change = Change::from_bytes(bytes.to_vec()).map_err(|_| ReplError::Malformed)?;
    if change.hash().0 != hash
        || change.raw_bytes() != bytes
        || change.actor_id().to_bytes() != seed_actor(origin.doc_id())
        || change.seq() != 1
        || change.start_op().get() != 1
        || !change.deps().is_empty()
        || change.timestamp() != 0
        || change.message().is_some_and(|s| !s.is_empty())
        || !change.extra_bytes().is_empty()
        || change.is_empty()
    {
        return Err(ReplError::Malformed);
    }
    if change
        .decode()
        .operations
        .iter()
        .any(|op| matches!(&op.key, automerge::legacy::Key::Map(key) if key.starts_with("_p1/op/")))
    {
        return Err(ReplError::Malformed);
    }
    Ok(change)
}

fn reject_markers(doc: &AutoCommit) -> Result<(), ReplError> {
    if doc.keys(ROOT).any(|key| key.starts_with("_p1/op/")) {
        return Err(ReplError::Malformed);
    }
    Ok(())
}

pub(crate) fn am_error(error: automerge::AutomergeError) -> ReplError {
    ReplError::Automerge(error.to_string())
}
