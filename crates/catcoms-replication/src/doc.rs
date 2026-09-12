//! [`EncryptedDoc`]; one encrypted, replicated CRDT document.
//!
//! A document (a channel, wiki page, status feed, calendar) is an automerge
//! document plus an append-only log of the [`SignedOp`]s that built it. Local
//! edits produce a [`SealedOp`] to broadcast; inbound sealed ops are decrypted,
//! their inner signature verified, then applied (automerge buffers any that
//! arrive before their dependencies). For a member who joined late, a current
//! member exports the signed-op log **re-sealed under the current epoch**, so the
//! latecomer converges without ever needing old epoch keys (forward secrecy is
//! preserved) while still verifying each op's original authorship.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;

use automerge::transaction::Transactable;
use automerge::{ActorId, AutoCommit, Change, ChangeHash, ReadDoc, ScalarValue, Value, ROOT};
use catcoms_crypto::DeviceId;
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_rt::CryptoRngCore;
use catcoms_storage::pad;
use catcoms_wire::{Decoder, DocType, Encoder};

use crate::epoch::{
    Admission, AdmittedOperation, DomainOp, EpochGate, LogicalDocument, MAX_SIGNED_EPOCH_OP_BYTES,
};
use crate::op::{SealedOp, SignedOp};
use crate::ReplError;
use crate::{CheckpointOrigin, VerifiedCheckpoint};

/// Cap on how many changes one [`EncryptedDoc::holders_of`] query may ask about; each
/// target takes one bit of the propagation mask the single DAG pass carries.
pub const MAX_DELIVERY_TARGETS: usize = 64;

/// Authenticated metadata about one newly applied remote operation.
///
/// The sync layer uses this only after [`EncryptedDoc`] has decrypted the sealed frame, verified
/// the inner device signature and accepted the Automerge change. Exposing the author and stable
/// change hash here avoids trying to infer either identity from the gossipsub forwarding peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedOp {
    pub author_device: DeviceId,
    pub change: ChangeHash,
}

/// One encrypted, replicated CRDT document.
pub struct EncryptedDoc {
    doc_type: DocType,
    doc_id: u128,
    doc: AutoCommit,
    log: Vec<SignedOp>,
    applied: HashSet<[u8; 32]>,
    /// `automerge change hash → the device that signed the op carrying it`, for the delivery
    /// query ([`EncryptedDoc::holders_of`]). Attribution comes from the **signed** op envelope,
    /// not the change's automerge actor id, so a member cannot forge a change that looks like
    /// another member's. Built lazily and incrementally from `log` (see `index_authors`), so a
    /// caller that never asks about delivery pays nothing. Derived state; never persisted.
    change_authors: HashMap<ChangeHash, DeviceId>,
    /// How many entries of `log` are already reflected in `change_authors`.
    authors_indexed: usize,
    /// The last [`EncryptedDoc::snapshot`] and the exact document state it describes: the
    /// automerge heads plus the op-log length, which together are everything the serialization
    /// reads. Persistence re-serializes every open document of a server on every write, so an
    /// unchanged document was re-encoded once per sent message; this makes it pay once per
    /// change instead. Not a correctness shortcut: a key that has not moved means the bytes
    /// cannot have. Held only while under [`MAX_CACHED_SNAPSHOT_BYTES`], so the largest
    /// documents (whose bytes are the ones worth not duplicating) are simply re-encoded.
    /// Derived state; never persisted.
    snapshot_cache: Option<(SnapshotKey, Vec<u8>)>,
    /// Receipt-authorized seed identity; absent for epoch zero and legacy documents. Its raw
    /// change is in Automerge, not the signed user-op log, and must survive vault restore.
    checkpoint: Option<CheckpointOrigin>,
}

/// What a serialization of a document depends on: its automerge heads and its op-log length.
type SnapshotKey = (Vec<ChangeHash>, usize);

/// Largest snapshot retained by [`EncryptedDoc::snapshot`]'s cache. Above this the saving is not
/// worth a second copy of the document in memory; a busy channel re-encodes as it always did.
pub const MAX_CACHED_SNAPSHOT_BYTES: usize = 1 << 20;

impl EncryptedDoc {
    /// Create an empty document. `actor` (this device) becomes the automerge
    /// actor id, so changes are deterministically attributed.
    pub fn new(doc_type: DocType, doc_id: u128, actor: &DeviceId) -> Self {
        let mut doc = AutoCommit::new();
        doc.set_actor(ActorId::from(actor.as_bytes().to_vec()));
        Self {
            doc_type,
            doc_id,
            doc,
            log: Vec::new(),
            applied: HashSet::new(),
            change_authors: HashMap::new(),
            authors_indexed: 0,
            snapshot_cache: None,
            checkpoint: None,
        }
    }

    /// Open a checkpoint only after receipt and typed projection verification. This creates a
    /// separate DAG, rebinds the local writer, and leaves the source epoch untouched; settlement
    /// must persist excluded content before replacing its own current-document pointer.
    pub fn from_checkpoint(
        checkpoint: &VerifiedCheckpoint,
        actor: &DeviceId,
    ) -> Result<Self, ReplError> {
        let origin = checkpoint.origin();
        let change = crate::checkpoint::validate_change(origin, checkpoint.bytes())?;
        let mut result = Self::new(origin.document().doc_type, origin.doc_id(), actor);
        result
            .doc
            .apply_changes([change])
            .map_err(crate::checkpoint::am_error)?;
        result.checkpoint = Some(origin.clone());
        Ok(result)
    }

    /// The authenticated origin required to exclude the one seed from close accounting.
    pub fn checkpoint_origin(&self) -> Option<&CheckpointOrigin> {
        self.checkpoint.as_ref()
    }

    /// Borrow the complete accepted log for the bounded registry restart format. Its order is
    /// dependency-complete because registry admission refuses unavailable predecessors.
    pub(crate) fn signed_log(&self) -> &[SignedOp] {
        &self.log
    }

    /// Rebuild an authenticated vault log, not an independently serialized Automerge image.
    /// Historical roster/share exemptions have already been admitted and are checked against
    /// the saved gate by the coordinator. Recheck signatures, exact semantics and dependencies;
    /// no absent, unsigned or queued change may contribute to the reconstructed projection.
    pub(crate) fn restore_domain_log<V>(
        &mut self,
        logical: &LogicalDocument,
        operations: Vec<SignedOp>,
        mut validate: V,
    ) -> Result<Vec<AdmittedOperation>, ReplError>
    where
        V: FnMut(&DomainOp, &Change, &AutoCommit, bool) -> Result<(), ReplError>,
    {
        if !self.log.is_empty() || operations.len() > crate::epoch::MAX_EPOCH_OPERATIONS {
            return Err(ReplError::EpochBound);
        }
        let mut total = 0usize;
        let mut metadata = Vec::new();
        let mut ids = HashSet::new();
        for op in operations {
            self.check_doc(op.doc_type, op.doc_id)?;
            let encoded_len = op.encode().len();
            total = total.saturating_add(encoded_len);
            if encoded_len > MAX_SIGNED_EPOCH_OP_BYTES || total > crate::epoch::MAX_EPOCH_BYTES {
                return Err(ReplError::EpochBound);
            }
            if !op.verify() {
                return Err(ReplError::BadSignature);
            }
            let domain = op.parsed_domain_op()?.ok_or(ReplError::Malformed)?;
            if domain.doc_type != logical.doc_type || domain.logical_key != logical.logical_key {
                return Err(ReplError::EpochScope);
            }
            let change = Change::from_bytes(op.delta.clone()).map_err(|_| ReplError::Malformed)?;
            if change.actor_id().to_bytes() != op.author_device.as_bytes() {
                return Err(ReplError::EpochAuthority);
            }
            let domain_op_id = domain.id(&op.author_device);
            // Presence comes from Automerge's accepted change graph, not a saved index.
            // Reconstructing each predecessor's raw operations would add historical scans just
            // to answer existence. Metadata is sufficient here: this graph starts with only a
            // verified seed, and each accepted predecessor has passed this loop's full checks.
            if !ids.insert(domain_op_id)
                || self.applied.contains(&op.hash())
                || self.doc.get_change_meta_by_hash(&change.hash()).is_some()
                || (self.checkpoint.is_some() && change.deps().is_empty())
                || change
                    .deps()
                    .iter()
                    .any(|h| self.doc.get_change_meta_by_hash(h).is_none())
            {
                return Err(ReplError::Malformed);
            }
            // Historical reads perform causal visibility work for each property. When a change
            // depends on the ENTIRE actual frontier, its causal view is exactly the current
            // committed view. Derive that fact here, after authentication/dependency checks,
            // never from saved metadata or a peer assertion. No mutation occurs before the
            // immutable validator uses it. Concurrent/older branches still use historical reads.
            let mut deps = change.deps().to_vec();
            deps.sort_unstable();
            let current_view = self.doc.get_heads() == deps;
            validate(&domain, &change, &self.doc, current_view)?;
            self.doc
                .apply_changes([change])
                .map_err(crate::checkpoint::am_error)?;
            if !self.has_domain_marker(&domain_op_id)? {
                return Err(ReplError::Malformed);
            }
            metadata.push(AdmittedOperation {
                op_hash: op.hash(),
                domain_op_id,
                author: op.author_device,
                encoded_len,
            });
            self.record(op);
        }
        Ok(metadata)
    }

    /// Serve the checkpoint's raw seed by hash without duplicating it in the user-op log.
    pub fn checkpoint_bytes(&mut self) -> Result<Option<Vec<u8>>, ReplError> {
        self.checkpoint
            .as_ref()
            .map(|origin| {
                self.doc
                    .get_change_by_hash(&ChangeHash(origin.seed_hash()))
                    .map(|change| change.raw_bytes().to_vec())
                    .ok_or(ReplError::Malformed)
            })
            .transpose()
    }

    /// Reconstruct only a previously verified closure, preserving the one checkpoint seed.
    /// This is a read-only projection workspace and never inherits the source's excluded heads.
    pub(crate) fn projection_for_closure(
        &mut self,
        operations: &[SignedOp],
    ) -> Result<AutoCommit, ReplError> {
        let mut projection = AutoCommit::new().with_actor(ActorId::from(vec![0; 32]));
        if let Some(seed) = self.checkpoint_bytes()? {
            projection
                .apply_changes([Change::from_bytes(seed).map_err(|_| ReplError::Malformed)?])
                .map_err(crate::checkpoint::am_error)?;
        }
        for op in operations {
            projection
                .apply_changes([
                    Change::from_bytes(op.delta.clone()).map_err(|_| ReplError::Malformed)?
                ])
                .map_err(crate::checkpoint::am_error)?;
        }
        Ok(projection)
    }

    /// Borrow the underlying automerge document (for reads/projection).
    pub fn doc(&self) -> &AutoCommit {
        &self.doc
    }

    /// This document's type (for re-keying a restored doc in the sync layer's map).
    pub fn doc_type(&self) -> DocType {
        self.doc_type
    }

    /// This document's id.
    pub fn doc_id(&self) -> u128 {
        self.doc_id
    }

    /// Number of ops in this document's log.
    pub fn op_count(&self) -> usize {
        self.log.len()
    }

    /// Whether this epoch already contains the author-bound marker for a domain operation.
    /// Replay callers use this before rebuilding an intent; materializers ignore marker keys.
    pub fn has_domain_marker(&self, op_id: &[u8; 32]) -> Result<bool, ReplError> {
        self.doc
            .get(ROOT, domain_marker_key(op_id))
            .map_err(|e| ReplError::Automerge(e.to_string()))
            .map(|value| {
                value.is_some_and(|(value, _)| {
                    matches!(value, Value::Scalar(value) if value.as_ref() == &ScalarValue::Uint(1))
                })
            })
    }

    /// Current Automerge heads as raw protocol hashes, sorted for a canonical P1 close record.
    pub fn heads(&mut self) -> Vec<[u8; 32]> {
        let mut heads: Vec<[u8; 32]> = self
            .doc
            .get_heads()
            .into_iter()
            .map(|hash| hash.0)
            .collect();
        heads.sort_unstable();
        heads
    }

    /// What this node tells a serving peer it already holds, for an incremental catch-up: its
    /// automerge frontier plus the immediate ancestors of that frontier, deduplicated and capped
    /// at `max` (newest first, so a truncated list still describes the most useful part).
    ///
    /// The ancestors matter because a frontier alone is fragile in the one case that matters: a
    /// member that wrote while it could not reach anyone has a head nobody else has seen, and a
    /// peer that cannot see a hash cannot subtract anything behind it, so the whole history would
    /// come back. Naming the parents of that local change gives the peer a hash it does know.
    ///
    /// Sound by the same argument for every hash listed: holding a change means holding its
    /// dependencies, so a peer may treat anything causally behind a named hash as already held.
    pub fn sync_frontier(&mut self, max: usize) -> Vec<[u8; 32]> {
        let heads = self.doc.get_heads();
        let mut out: Vec<[u8; 32]> = Vec::new();
        let mut seen: HashSet<ChangeHash> = HashSet::new();
        let ancestors: Vec<ChangeHash> = heads
            .iter()
            .filter_map(|hash| self.doc.get_change_by_hash(hash))
            .flat_map(|change| change.deps().to_vec())
            .collect();
        for hash in heads.into_iter().chain(ancestors) {
            if out.len() >= max {
                break;
            }
            if seen.insert(hash) {
                out.push(hash.0);
            }
        }
        out
    }

    /// Return the signed operations in the dependency-closed history selected by `heads`.
    ///
    /// P1 close validation uses this instead of trusting a close author's operation count. A
    /// checkpoint seed is the sole permitted unsigned Automerge change; callers name its exact
    /// hash in `unsigned_seed`. Every other change in the selected closure must map to exactly one
    /// signed P1 operation.
    pub(crate) fn signed_ops_for_heads(
        &mut self,
        heads: &[[u8; 32]],
        unsigned_seed: Option<[u8; 32]>,
    ) -> Result<Vec<SignedOp>, ReplError> {
        if unsigned_seed != self.checkpoint.as_ref().map(CheckpointOrigin::seed_hash) {
            return Err(ReplError::EpochScope);
        }
        // Walk the named closure explicitly. `AutoCommit::fork_at` would select the same graph but
        // deliberately creates a random actor id, which is both unnecessary for a read-only walk
        // and outside Mewtual's injected RNG seam.
        let mut wanted = BTreeSet::new();
        let mut stack: Vec<ChangeHash> = heads.iter().copied().map(ChangeHash).collect();
        while let Some(hash) = stack.pop() {
            if !wanted.insert(hash.0) {
                continue;
            }
            let change = self
                .doc
                .get_change_by_hash(&hash)
                .ok_or(ReplError::Malformed)?;
            stack.extend(change.deps().iter().copied());
        }
        if let Some(seed) = unsigned_seed {
            if !wanted.remove(&seed) {
                return Err(ReplError::Malformed);
            }
        }

        let selected_hashes = wanted.clone();
        let mut operation_by_change = std::collections::BTreeMap::new();
        for op in &self.log {
            let change = Change::from_bytes(op.delta.clone())
                .map_err(|e| ReplError::Automerge(e.to_string()))?
                .hash()
                .0;
            if selected_hashes.contains(&change) {
                if op.domain_op.is_none() {
                    return Err(ReplError::Malformed);
                }
                if operation_by_change.insert(change, op.clone()).is_some() {
                    // Two signed envelopes claiming one Automerge change make authorship and
                    // per-device accounting ambiguous. P1's author-bound marker prevents this for a
                    // valid operation, so a duplicate claim is malformed rather than tie-broken.
                    return Err(ReplError::Malformed);
                }
            }
        }
        if operation_by_change.len() != selected_hashes.len() {
            return Err(ReplError::Malformed);
        }

        // Produce a canonical dependency order. The append log reflects arrival order, which can
        // differ across peers, and therefore cannot be used as checkpoint input directly.
        let mut dependency_count = std::collections::BTreeMap::new();
        let mut dependents: std::collections::BTreeMap<[u8; 32], Vec<[u8; 32]>> =
            std::collections::BTreeMap::new();
        for hash in &selected_hashes {
            let change = self
                .doc
                .get_change_by_hash(&ChangeHash(*hash))
                .ok_or(ReplError::Malformed)?;
            let mut count = 0usize;
            for dependency in change.deps() {
                if selected_hashes.contains(&dependency.0) {
                    count += 1;
                    dependents.entry(dependency.0).or_default().push(*hash);
                }
            }
            dependency_count.insert(*hash, count);
        }
        let mut ready: BTreeSet<[u8; 32]> = dependency_count
            .iter()
            .filter_map(|(hash, count)| (*count == 0).then_some(*hash))
            .collect();
        let mut out = Vec::with_capacity(selected_hashes.len());
        while let Some(hash) = ready.pop_first() {
            out.push(
                operation_by_change
                    .remove(&hash)
                    .ok_or(ReplError::Malformed)?,
            );
            for dependent in dependents.get(&hash).into_iter().flatten() {
                let count = dependency_count
                    .get_mut(dependent)
                    .ok_or(ReplError::Malformed)?;
                *count = count.checked_sub(1).ok_or(ReplError::Malformed)?;
                if *count == 0 {
                    ready.insert(*dependent);
                }
            }
        }
        if !operation_by_change.is_empty() {
            return Err(ReplError::Malformed);
        }
        Ok(out)
    }

    /// Which devices **provably hold** each of `targets` (automerge change hashes), from the
    /// document alone; the read-only half of the delivery-state query.
    ///
    /// A device `D` counts for target `C` when `D` authored some change whose causal history
    /// contains `C`: `D` could not have built on `C` without holding it, and the change carrying
    /// that proof is signed by `D`. This is the same predicate the design's `their_heads` route
    /// describes ("the peer's confirmed heads causally include the op"), evaluated against
    /// evidence already in the doc rather than against a sync session; Mewtual replicates by
    /// broadcasting sealed ops, so no per-peer automerge sync state exists to read.
    ///
    /// The result is *sound but incomplete*: a device that received `C` and has not written since
    /// leaves no evidence and is simply absent. Callers must render absence as "unknown", never
    /// as "not delivered". Returns one entry per element of `targets` (sorted, deduped); targets
    /// past [`MAX_DELIVERY_TARGETS`] always come back empty.
    pub fn holders_of(&mut self, targets: &[ChangeHash]) -> Vec<Vec<DeviceId>> {
        let mut out = vec![Vec::new(); targets.len()];
        let n = targets.len().min(MAX_DELIVERY_TARGETS);
        if n == 0 {
            return out;
        }
        self.index_authors();
        let mut bit_of: HashMap<ChangeHash, u64> = HashMap::with_capacity(n);
        for (i, h) in targets[..n].iter().enumerate() {
            *bit_of.entry(*h).or_default() |= 1u64 << i;
        }
        // One pass over the change DAG. `get_changes_meta(&[])` yields every change in the order
        // it entered the graph, and automerge only admits a change once all its dependencies are
        // present; so dependencies are always visited before dependents and `carried` is complete
        // by the time it is read. If that ever stopped holding, a dep would simply be missing from
        // the map and the mask would lose a bit: an under-count (silence), never a false claim.
        let mut carried: HashMap<ChangeHash, u64> = HashMap::new();
        let mut by_device: HashMap<DeviceId, u64> = HashMap::new();
        for meta in self.doc.get_changes_meta(&[]) {
            let mut mask = bit_of.get(&meta.hash).copied().unwrap_or(0);
            for dep in &meta.deps {
                mask |= carried.get(dep).copied().unwrap_or(0);
            }
            if mask != 0 {
                // Only changes at or after a target can carry bits, so the map stays proportional
                // to the recent tail of history rather than to the whole document (a missing
                // entry reads as 0, which is exactly what a pruned change means).
                carried.insert(meta.hash, mask);
                if let Some(device) = self.change_authors.get(&meta.hash) {
                    *by_device.entry(*device).or_default() |= mask;
                }
            }
        }
        for (device, mask) in by_device {
            for (i, slot) in out.iter_mut().enumerate().take(n) {
                if mask & (1u64 << i) != 0 {
                    slot.push(device);
                }
            }
        }
        for slot in &mut out {
            slot.sort_unstable();
        }
        out
    }

    /// Bring `change_authors` up to date with `log`. Each op's `delta` is exactly the one
    /// automerge change [`EncryptedDoc::edit_tracked`] produced, so parsing it recovers that
    /// change's hash and pairs it with the op's signature-verified author. Incremental: every op
    /// is parsed at most once, and only if a delivery query is ever made.
    fn index_authors(&mut self) {
        for i in self.authors_indexed..self.log.len() {
            let (delta, author) = {
                let op = &self.log[i];
                (op.delta.clone(), op.author_device)
            };
            // A malformed or multi-change delta simply goes unattributed (it can still carry
            // other members' evidence forward through the DAG pass; it just proves nothing).
            if let Ok(change) = Change::from_bytes(delta) {
                self.change_authors.insert(change.hash(), author);
            }
        }
        self.authors_indexed = self.log.len();
    }

    /// Serialize this document for persistence (Phase 9d): the materialized automerge state
    /// plus the signed-op log (the log carries the per-op signatures the automerge state
    /// does not, so a restored member can still serve catch-up). The `applied` dedup set is
    /// rebuilt from the log on restore. **Secret**; holds plaintext document content; the
    /// persistence layer seals it under `db_key` before it touches disk.
    pub fn snapshot(&mut self) -> Result<Vec<u8>, ReplError> {
        let key: SnapshotKey = (self.doc.get_heads(), self.log.len());
        if let Some((cached_key, bytes)) = &self.snapshot_cache {
            if *cached_key == key {
                return Ok(bytes.clone());
            }
        }
        let bytes = self.encode_snapshot()?;
        self.snapshot_cache =
            (bytes.len() <= MAX_CACHED_SNAPSHOT_BYTES).then(|| (key, bytes.clone()));
        Ok(bytes)
    }

    /// The serialization itself, always recomputed. See [`EncryptedDoc::snapshot`].
    fn encode_snapshot(&mut self) -> Result<Vec<u8>, ReplError> {
        let doc_bytes = self.doc.save();
        let count = u32::try_from(self.log.len()).map_err(|_| ReplError::Malformed)?;
        let mut e = Encoder::new();
        e.put_u16(self.doc_type.tag());
        e.put_u128(self.doc_id);
        e.put_bytes(&doc_bytes).map_err(|_| ReplError::Malformed)?;
        e.put_u32(count);
        for op in &self.log {
            e.put_bytes(&op.encode())
                .map_err(|_| ReplError::Malformed)?;
        }
        // Legacy/epoch-zero snapshots remain byte-for-byte unchanged. The optional extension is
        // local vault format only; new checkpoints cannot be interpreted as seedless snapshots.
        if let Some(origin) = &self.checkpoint {
            e.put_u8(1);
            e.put_bytes(&origin.encode()?)
                .map_err(|_| ReplError::Malformed)?;
        }
        Ok(e.finish())
    }

    /// Reconstruct a document from a [`EncryptedDoc::snapshot`] blob.
    pub fn restore(bytes: &[u8]) -> Result<Self, ReplError> {
        let mut d = Decoder::new(bytes);
        let tag = d.get_u16().map_err(|_| ReplError::Malformed)?;
        let doc_type = DocType::from_tag(tag).ok_or(ReplError::Malformed)?;
        let doc_id = d.get_u128().map_err(|_| ReplError::Malformed)?;
        let doc_bytes = d.get_bytes().map_err(|_| ReplError::Malformed)?;
        let doc = AutoCommit::load(doc_bytes).map_err(|e| ReplError::Automerge(e.to_string()))?;
        let count = d.get_u32().map_err(|_| ReplError::Malformed)?;
        let mut log = Vec::new();
        let mut applied = HashSet::new();
        for _ in 0..count {
            let op = SignedOp::decode(d.get_bytes().map_err(|_| ReplError::Malformed)?)?;
            applied.insert(op.hash());
            log.push(op);
        }
        let checkpoint = if d.is_empty() {
            None
        } else {
            if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
                return Err(ReplError::Malformed);
            }
            let origin =
                CheckpointOrigin::decode(d.get_bytes().map_err(|_| ReplError::Malformed)?)?;
            if origin.document().doc_type != doc_type || origin.doc_id() != doc_id {
                return Err(ReplError::EpochScope);
            }
            let seed = doc
                .get_change_by_hash(&ChangeHash(origin.seed_hash()))
                .ok_or(ReplError::Malformed)?;
            crate::checkpoint::validate_change(&origin, seed.raw_bytes())?;
            Some(origin)
        };
        d.finish().map_err(|_| ReplError::Malformed)?;
        Ok(Self {
            doc_type,
            doc_id,
            doc,
            log,
            applied,
            change_authors: HashMap::new(),
            authors_indexed: 0,
            snapshot_cache: None,
            checkpoint,
        })
    }

    /// Restore and bind all subsequently authored changes to this device's verified identity.
    ///
    /// P1 requires the Automerge actor on every change to equal the signed envelope author. A
    /// loaded `AutoCommit` otherwise owns an implementation-selected local actor, so callers that
    /// may edit after restart must use this entry point rather than read-only [`Self::restore`].
    pub fn restore_for_actor(bytes: &[u8], actor: &DeviceId) -> Result<Self, ReplError> {
        let mut restored = Self::restore(bytes)?;
        restored
            .doc
            .set_actor(ActorId::from(actor.as_bytes().to_vec()));
        Ok(restored)
    }

    /// Apply a local edit, returning a [`SealedOp`] to broadcast. The closure
    /// mutates the automerge document; the resulting change is signed and sealed.
    pub fn edit<F>(
        &mut self,
        device: &MlsDevice,
        group: &ServerGroup,
        rng: &mut impl CryptoRngCore,
        edit: F,
    ) -> Result<SealedOp, ReplError>
    where
        F: FnOnce(&mut AutoCommit) -> Result<(), automerge::AutomergeError>,
    {
        self.edit_tracked(device, group, rng, edit)
            .map(|(op, _)| op)
    }

    /// [`EncryptedDoc::edit`], also returning the **automerge change hash** the edit produced;
    /// the stable, content-addressed handle a caller needs to later ask [`EncryptedDoc::holders_of`]
    /// who has received this particular edit.
    pub fn edit_tracked<F>(
        &mut self,
        device: &MlsDevice,
        group: &ServerGroup,
        rng: &mut impl CryptoRngCore,
        edit: F,
    ) -> Result<(SealedOp, ChangeHash), ReplError>
    where
        F: FnOnce(&mut AutoCommit) -> Result<(), automerge::AutomergeError>,
    {
        if is_epoch_managed(self.doc_type) {
            // New P1 types have no legacy writers. A v1 operation here would bypass durable
            // intents, semantic validation and the shared epoch lifecycle gate.
            return Err(ReplError::EpochScope);
        }
        edit(&mut self.doc).map_err(|e| ReplError::Automerge(e.to_string()))?;
        self.doc.commit();
        let change = self
            .doc
            .get_last_local_change()
            .ok_or(ReplError::NoChange)?;
        let hash = change.hash();
        let delta = change.raw_bytes().to_vec();

        let op = SignedOp::sign(device, self.doc_type, self.doc_id, delta)?;
        let sealed = SealedOp::seal(&op, group, device, rng)?;
        self.record(op);
        Ok((sealed, hash))
    }

    /// Author a P1 operation through the epoch lifecycle gate.
    ///
    /// The caller must durably persist the corresponding local intent before entering this
    /// method. The change is built, signed and sealed on a rollback-safe clone, then admitted by
    /// the same gate
    /// used by receipt settlement. Losing a seal race therefore leaves the live document and log
    /// untouched so the durable intent can render as an overlay and replay in the successor.
    #[allow(clippy::too_many_arguments)]
    pub fn edit_domain_gated<F, V>(
        &mut self,
        logical_document: &LogicalDocument,
        gate: &EpochGate,
        device: &MlsDevice,
        group: &ServerGroup,
        rng: &mut impl CryptoRngCore,
        domain_op: &DomainOp,
        edit: F,
        validate_change: V,
    ) -> Result<(SealedOp, ChangeHash), ReplError>
    where
        F: FnOnce(&mut AutoCommit) -> Result<(), automerge::AutomergeError>,
        V: FnOnce(&DomainOp, &Change) -> Result<(), ReplError>,
    {
        self.edit_domain_preflight_gated(
            logical_document,
            gate,
            device,
            group,
            rng,
            domain_op,
            edit,
            validate_change,
            |_| Ok(()),
        )
    }

    /// Typed P1 edit with a rollback-safe preflight of the entire prospective projection.
    /// Consumers encode their exact next checkpoint here, before admission or publication.
    #[allow(clippy::too_many_arguments)]
    pub fn edit_domain_preflight_gated<F, V, P>(
        &mut self,
        logical_document: &LogicalDocument,
        gate: &EpochGate,
        device: &MlsDevice,
        group: &ServerGroup,
        rng: &mut impl CryptoRngCore,
        domain_op: &DomainOp,
        edit: F,
        validate_change: V,
        preflight: P,
    ) -> Result<(SealedOp, ChangeHash), ReplError>
    where
        F: FnOnce(&mut AutoCommit) -> Result<(), automerge::AutomergeError>,
        V: FnOnce(&DomainOp, &Change) -> Result<(), ReplError>,
        P: FnOnce(&AutoCommit) -> Result<(), ReplError>,
    {
        if logical_document.doc_type != self.doc_type
            || logical_document.server_id != group.group_id()
            || domain_op.doc_type != self.doc_type
            || domain_op.logical_key != logical_document.logical_key
        {
            return Err(ReplError::EpochScope);
        }
        gate.verify_scope(logical_document, self.doc_id)?;
        self.verify_checkpoint_scope(logical_document, gate)?;
        if group.member_signature_key(&device.device_id()).as_deref()
            != Some(device.public_key_bytes().as_slice())
        {
            return Err(ReplError::EpochAuthority);
        }
        if self.has_domain_marker(&domain_op.id(&device.device_id()))? {
            return Err(ReplError::NoChange);
        }
        // `fork()` randomizes the actor; a clone gives us a rollback-safe staging graph while
        // preserving the authenticated device actor for the new change.
        let mut staged = self.doc.clone();
        edit(&mut staged).map_err(|e| ReplError::Automerge(e.to_string()))?;
        let marker = domain_marker_key(&domain_op.id(&device.device_id()));
        staged
            .put(ROOT, marker, 1u64)
            .map_err(|e| ReplError::Automerge(e.to_string()))?;
        staged.commit();
        let change = staged.get_last_local_change().ok_or(ReplError::NoChange)?;
        validate_change(domain_op, &change)?;
        if change.actor_id().to_bytes() != device.device_id().as_bytes() {
            return Err(ReplError::EpochAuthority);
        }
        preflight(&staged)?;
        let change_hash = change.hash();
        let op = SignedOp::sign_domain(
            device,
            self.doc_type,
            self.doc_id,
            change.raw_bytes().to_vec(),
            domain_op,
        )?;
        let sealed = SealedOp::seal(&op, group, device, rng)?;
        let admitted = AdmittedOperation {
            op_hash: op.hash(),
            domain_op_id: domain_op.id(&op.author_device),
            author: op.author_device,
            encoded_len: op.encode().len(),
        };
        let admission = gate.admit_local_and_commit(admitted, || {
            self.doc = staged;
            self.record(op);
        })?;
        if admission != Admission::Accepted {
            // A locally constructed change is based on the current graph and uses a fresh nonce;
            // finding its exact envelope in the gate but not this log is an atomicity violation.
            return Err(ReplError::Malformed);
        }
        Ok((sealed, change_hash))
    }

    /// Decrypt, authenticate and conditionally apply one P1 operation through an epoch gate.
    ///
    /// An operation that loses the receipt-seal race is authenticated and charged only to the
    /// bounded quarantine; its Automerge bytes never enter live or durable document state.
    pub fn ingest_domain_gated<V>(
        &mut self,
        logical_document: &LogicalDocument,
        gate: &EpochGate,
        sealed: &SealedOp,
        group: &ServerGroup,
        device: &MlsDevice,
        validate_change: V,
    ) -> Result<Admission, ReplError>
    where
        V: FnOnce(&DomainOp, &Change) -> Result<(), ReplError>,
    {
        self.ingest_domain_preflight_gated(
            logical_document,
            gate,
            sealed,
            group,
            device,
            validate_change,
            |_| Ok(()),
        )
    }

    /// Inbound counterpart of [`Self::edit_domain_preflight_gated`]; a remote change cannot bypass
    /// the exact checkpoint-size and schema preflight used by the editor.
    #[allow(clippy::too_many_arguments)]
    pub fn ingest_domain_preflight_gated<V, P>(
        &mut self,
        logical_document: &LogicalDocument,
        gate: &EpochGate,
        sealed: &SealedOp,
        group: &ServerGroup,
        device: &MlsDevice,
        validate_change: V,
        preflight: P,
    ) -> Result<Admission, ReplError>
    where
        V: FnOnce(&DomainOp, &Change) -> Result<(), ReplError>,
        P: FnOnce(&AutoCommit) -> Result<(), ReplError>,
    {
        self.check_doc(sealed.doc_type, sealed.doc_id)?;
        if logical_document.server_id != group.group_id() {
            return Err(ReplError::EpochScope);
        }
        gate.verify_scope(logical_document, self.doc_id)?;
        self.verify_checkpoint_scope(logical_document, gate)?;
        if sealed.epoch != group.epoch() {
            return Err(ReplError::EpochUnavailable(sealed.epoch));
        }
        let key = group.channel_secret(device, self.doc_type, self.doc_id)?;
        let op = sealed.open(&key)?;
        // Possession of the group sealing key authenticates the relay, not the inner author.
        // New open-epoch content must come from an admitted device so identity churn cannot
        // mint fresh per-device shares. Previously accepted history remains valid after removal;
        // historical seed/close authorization is a separate receipt-bound path.
        if !self.applied.contains(&op.hash())
            && group.member_signature_key(&op.author_device).as_deref()
                != Some(op.author_pubkey.as_slice())
        {
            return Err(ReplError::EpochAuthority);
        }
        self.apply_domain_gated(logical_document, gate, op, validate_change, preflight)
    }

    fn apply_domain_gated<V, P>(
        &mut self,
        logical_document: &LogicalDocument,
        gate: &EpochGate,
        op: SignedOp,
        validate_change: V,
        preflight: P,
    ) -> Result<Admission, ReplError>
    where
        V: FnOnce(&DomainOp, &Change) -> Result<(), ReplError>,
        P: FnOnce(&AutoCommit) -> Result<(), ReplError>,
    {
        self.check_doc(op.doc_type, op.doc_id)?;
        gate.verify_scope(logical_document, self.doc_id)?;
        let op_hash = op.hash();
        if self.applied.contains(&op_hash) {
            return Ok(Admission::Duplicate);
        }
        if !op.verify() {
            return Err(ReplError::BadSignature);
        }
        let encoded_len = op.encode().len();
        if encoded_len > MAX_SIGNED_EPOCH_OP_BYTES {
            return Err(ReplError::EpochBound);
        }
        let domain_op = op.parsed_domain_op()?.ok_or(ReplError::Malformed)?;
        if logical_document.doc_type != self.doc_type
            || domain_op.doc_type != self.doc_type
            || domain_op.logical_key != logical_document.logical_key
        {
            return Err(ReplError::EpochScope);
        }
        let change = Change::from_bytes(op.delta.clone())
            .map_err(|e| ReplError::Automerge(e.to_string()))?;
        if change.actor_id().to_bytes() != op.author_device.as_bytes() {
            return Err(ReplError::EpochAuthority);
        }
        if self.checkpoint.is_some()
            && (change.deps().is_empty()
                || change
                    .deps()
                    .iter()
                    .any(|hash| self.doc.get_change_by_hash(hash).is_none()))
        {
            // Every accepted checkpoint edit must descend from the seed. Known descendants
            // preserve this inductively; an independent root or unavailable predecessor cannot
            // enter the document while its semantic projection is being checked.
            return Err(ReplError::EpochScope);
        }
        validate_change(&domain_op, &change)?;
        // Loading an inbound change authors nothing locally, so preserve the existing actor and
        // avoid `fork()`'s ambient random actor generation.
        let mut staged = self.doc.clone();
        staged
            .load_incremental(&op.delta)
            .map_err(|e| ReplError::Automerge(e.to_string()))?;
        let domain_op_id = domain_op.id(&op.author_device);
        let marker = domain_marker_key(&domain_op_id);
        let marker_is_one = staged
            .get(ROOT, marker)
            .map_err(|e| ReplError::Automerge(e.to_string()))?
            .is_some_and(|(value, _)| {
                matches!(value, Value::Scalar(value) if value.as_ref() == &ScalarValue::Uint(1))
            });
        if !marker_is_one {
            return Err(ReplError::Malformed);
        }
        preflight(&staged)?;
        let admission = gate.admit_inbound_and_commit(
            AdmittedOperation {
                op_hash,
                domain_op_id,
                author: op.author_device,
                encoded_len,
            },
            || {
                self.doc = staged;
                self.applied.insert(op_hash);
                self.log.push(op);
            },
        )?;
        match admission {
            Admission::Accepted => Ok(Admission::Accepted),
            Admission::Duplicate => {
                if self.has_domain_marker(&domain_op_id)? {
                    Ok(Admission::Duplicate)
                } else {
                    // Gate and document state are persisted atomically. A gate-only duplicate
                    // means the caller restored an inconsistent pair and must not silently skip.
                    Err(ReplError::Malformed)
                }
            }
            Admission::Quarantined | Admission::RejectedQuarantineFull => Ok(admission),
        }
    }

    /// Decrypt, verify and apply an inbound sealed op. Returns `true` if it was
    /// newly applied, `false` if it was a duplicate.
    pub fn ingest(
        &mut self,
        sealed: &SealedOp,
        group: &ServerGroup,
        device: &MlsDevice,
    ) -> Result<bool, ReplError> {
        self.ingest_tracked(sealed, group, device)
            .map(|applied| applied.is_some())
    }

    /// [`Self::ingest`], returning verified author/change metadata for a newly applied op.
    /// Duplicate ops return `None`, so callers cannot emit duplicate acknowledgements merely
    /// because gossipsub delivered the same ciphertext through more than one mesh edge.
    pub fn ingest_tracked(
        &mut self,
        sealed: &SealedOp,
        group: &ServerGroup,
        device: &MlsDevice,
    ) -> Result<Option<AppliedOp>, ReplError> {
        self.check_doc(sealed.doc_type, sealed.doc_id)?;
        if sealed.epoch != group.epoch() {
            return Err(ReplError::EpochUnavailable(sealed.epoch));
        }
        let key = group.channel_secret(device, self.doc_type, self.doc_id)?;
        let op = sealed.open(&key)?;
        self.apply_signed_tracked(op)
    }

    /// Decrypt, verify and apply an inbound sealed op using an **externally
    /// provided** channel key, *without* requiring the op's epoch to equal the
    /// group's current epoch. This is the entry point the sync layer uses for an
    /// op that was sealed under a just-superseded epoch and arrives after a
    /// membership commit advanced us: the caller supplies the channel key it
    /// retained for `sealed.epoch` (a bounded, zeroized past-epoch window).
    ///
    /// Confidentiality and authenticity are unchanged: a wrong key fails the AEAD
    /// open, and the op's inner author signature is still verified before it is
    /// applied; so this cannot be used to inject forged history. The caller pairs
    /// `key` with an epoch and passes that as `expected_epoch`; this asserts the op
    /// was actually sealed under it (defense in depth against a future refactor
    /// that mis-pairs key and epoch). Returns `true` if newly applied, `false` if
    /// it was a duplicate.
    pub fn ingest_with_key(
        &mut self,
        sealed: &SealedOp,
        expected_epoch: u64,
        key: &[u8; 32],
    ) -> Result<bool, ReplError> {
        self.ingest_with_key_tracked(sealed, expected_epoch, key)
            .map(|applied| applied.is_some())
    }

    /// [`Self::ingest_with_key`], returning verified metadata for a newly applied past-epoch op.
    pub fn ingest_with_key_tracked(
        &mut self,
        sealed: &SealedOp,
        expected_epoch: u64,
        key: &[u8; 32],
    ) -> Result<Option<AppliedOp>, ReplError> {
        self.check_doc(sealed.doc_type, sealed.doc_id)?;
        if sealed.epoch != expected_epoch {
            return Err(ReplError::EpochUnavailable(sealed.epoch));
        }
        let op = sealed.open(key)?;
        self.apply_signed_tracked(op)
    }

    /// Export the full signed-op log, re-sealed under the current epoch, so a
    /// late-joining member can catch up without old epoch keys.
    pub fn export_catchup(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<SealedOp>, ReplError> {
        let mut out = Vec::with_capacity(self.log.len());
        for op in &self.log {
            out.push(SealedOp::seal(op, group, device, rng)?);
        }
        Ok(out)
    }

    /// Export only the signed ops a requester holding `have_heads` is missing, re-sealed under
    /// the current epoch exactly as [`EncryptedDoc::export_catchup`] does.
    ///
    /// The requester names its automerge frontier; everything causally behind it is what it
    /// already has, so what remains is the difference. A reconnecting member therefore pays for
    /// the gap rather than for the whole history, which is what made rejoining a long-lived
    /// channel cost more every week it stayed alive.
    ///
    /// A head this node has never seen selects nothing: it is a change the *requester* has and
    /// this node does not, so it can exclude nothing here and the requester keeps it. An op whose
    /// delta will not parse is sent rather than withheld, because withholding could strand the
    /// requester and a duplicate is dropped on arrival.
    pub fn export_catchup_since(
        &mut self,
        have_heads: &[[u8; 32]],
        group: &ServerGroup,
        device: &MlsDevice,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<SealedOp>, ReplError> {
        let have = self.held_closure(have_heads);
        let mut out = Vec::new();
        for op in &self.log {
            let carried = Change::from_bytes(op.delta.clone()).ok().map(|c| c.hash());
            if carried.is_some_and(|hash| have.contains(&hash)) {
                continue;
            }
            out.push(SealedOp::seal(op, group, device, rng)?);
        }
        Ok(out)
    }

    /// [`Self::export_catchup_since`], resumable and bounded: begin at `from` in this node's log,
    /// take sealed operations until `budget` bytes are used, and report where to resume.
    ///
    /// The difference between this and capping the output of `export_catchup_since` is the whole
    /// point, and it is what makes a wide frontier survivable. That function recomputes the entire
    /// difference every call, so a caller that can only send a prefix of it sends the *same*
    /// prefix every time. When the frontier is truncated, that prefix is history the requester
    /// already holds, and the operations it actually needs sit behind a wall of duplicates it can
    /// never get past. Resuming by position means every exchange consumes log, so an all-duplicate
    /// page is still progress and the wall is finite.
    ///
    /// The log is append-only, so a position stays meaningful as the document grows. It is
    /// meaningful only against **this** node's log, though: the same operation sits at different
    /// positions on different members. Callers must not replay a position to a peer that did not
    /// issue it; the sync layer binds each one to the peer and the runtime that produced it.
    ///
    /// A page may legitimately be empty while still returning a resume point, because a run of
    /// operations the requester already holds is skipped rather than sent.
    pub fn export_catchup_page(
        &mut self,
        have_heads: &[[u8; 32]],
        from: usize,
        budget: usize,
        group: &ServerGroup,
        device: &MlsDevice,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(Vec<SealedOp>, Option<usize>), ReplError> {
        let have = self.held_closure(have_heads);
        let mut position = from.min(self.log.len());
        let mut out = Vec::new();
        let mut used = 0usize;
        while position < self.log.len() {
            let op = &self.log[position];
            let carried = Change::from_bytes(op.delta.clone()).ok().map(|c| c.hash());
            if carried.is_some_and(|hash| have.contains(&hash)) {
                position += 1;
                continue;
            }
            // The sealed size is deterministic from the unsealed one, so the budget is applied
            // before paying for the seal rather than after. Same accounting as the registry pager
            // and as the sync layer's own `size_capped_ops`: the padded body plus a 58-byte
            // envelope, the 4-byte pad footer, the 16-byte tag and 4 bytes of list framing.
            let bytes = pad::padded_len(op.encode().len(), pad::OP_PAD_FLOOR, pad::OP_PAD_CEILING)
                .saturating_add(82);
            if !out.is_empty() && used.saturating_add(bytes) > budget {
                break;
            }
            out.push(SealedOp::seal(op, group, device, rng)?);
            used = used.saturating_add(bytes);
            position += 1;
        }
        let next = (position < self.log.len()).then_some(position);
        Ok((out, next))
    }

    /// Every change at or behind `heads` that this node can actually resolve.
    ///
    /// A head this node has never seen selects nothing: it is a change the *requester* has and
    /// this node does not, so nothing can be excluded on its account and the requester keeps it.
    fn held_closure(&self, have_heads: &[[u8; 32]]) -> HashSet<ChangeHash> {
        let mut have: HashSet<ChangeHash> = HashSet::new();
        let mut stack: Vec<ChangeHash> = have_heads.iter().copied().map(ChangeHash).collect();
        while let Some(hash) = stack.pop() {
            let Some(change) = self.doc.get_change_by_hash(&hash) else {
                continue;
            };
            if !have.insert(hash) {
                continue;
            }
            stack.extend(change.deps().iter().copied());
        }
        have
    }

    /// Apply a catch-up bundle produced by [`EncryptedDoc::export_catchup`].
    /// Returns the number of newly applied ops.
    pub fn import_catchup(
        &mut self,
        ops: &[SealedOp],
        group: &ServerGroup,
        device: &MlsDevice,
    ) -> Result<usize, ReplError> {
        let (applied, terminal) = self.import_catchup_tracked(ops, group, device);
        terminal.map(|()| applied.len())
    }

    /// [`Self::import_catchup`], returning verified author/change metadata for every newly
    /// applied op. The sync owner uses this to acknowledge messages learned while it was offline;
    /// live gossip and catch-up must not have different delivery semantics.
    pub fn import_catchup_tracked(
        &mut self,
        ops: &[SealedOp],
        group: &ServerGroup,
        device: &MlsDevice,
    ) -> (Vec<AppliedOp>, Result<(), ReplError>) {
        let key = match group.channel_secret(device, self.doc_type, self.doc_id) {
            Ok(key) => key,
            Err(error) => return (Vec::new(), Err(error.into())),
        };
        let mut applied = Vec::new();
        for sealed in ops {
            if let Err(error) = self.check_doc(sealed.doc_type, sealed.doc_id) {
                return (applied, Err(error));
            }
            if sealed.epoch != group.epoch() {
                return (applied, Err(ReplError::EpochUnavailable(sealed.epoch)));
            }
            let op = match sealed.open(&key) {
                Ok(op) => op,
                Err(error) => return (applied, Err(error)),
            };
            match self.apply_signed_tracked(op) {
                Ok(Some(tracked)) => applied.push(tracked),
                Ok(None) => {}
                Err(error) => return (applied, Err(error)),
            }
        }
        (applied, Ok(()))
    }

    fn apply_signed_tracked(&mut self, op: SignedOp) -> Result<Option<AppliedOp>, ReplError> {
        self.check_doc(op.doc_type, op.doc_id)?;
        if is_epoch_managed(self.doc_type) {
            // Epoch-managed types must bind logical scope, semantic validation and lifecycle
            // admission through `ingest_domain_gated`.
            return Err(ReplError::EpochScope);
        }
        if op.domain_op.is_some() {
            return Err(ReplError::Malformed);
        }
        let hash = op.hash();
        if self.applied.contains(&hash) {
            return Ok(None);
        }
        if !op.verify() {
            return Err(ReplError::BadSignature);
        }
        // A locally authored SignedOp always carries exactly one Automerge change. Parse that
        // exact frame before moving it into the document so a receipt can name a stable hash;
        // malformed or multi-change bytes fail closed exactly as load_incremental would.
        let change = Change::from_bytes(op.delta.clone())
            .map_err(|e| ReplError::Automerge(e.to_string()))?
            .hash();
        let author_device = op.author_device;
        self.doc
            .load_incremental(&op.delta)
            .map_err(|e| ReplError::Automerge(e.to_string()))?;
        self.applied.insert(hash);
        self.log.push(op);
        Ok(Some(AppliedOp {
            author_device,
            change,
        }))
    }

    fn record(&mut self, op: SignedOp) {
        if self.applied.insert(op.hash()) {
            self.log.push(op);
        }
    }

    fn check_doc(&self, doc_type: DocType, doc_id: u128) -> Result<(), ReplError> {
        if doc_type != self.doc_type || doc_id != self.doc_id {
            return Err(ReplError::WrongDocument);
        }
        Ok(())
    }

    fn verify_checkpoint_scope(
        &self,
        logical: &LogicalDocument,
        gate: &EpochGate,
    ) -> Result<(), ReplError> {
        if let Some(origin) = &self.checkpoint {
            if origin.document() != logical || origin.epoch() != gate.epoch() {
                return Err(ReplError::EpochScope);
            }
        } else if gate.epoch() != 0
            || self.doc_id != crate::epoch_zero_id(logical.doc_type, &logical.logical_key)
        {
            // A caller cannot open an empty successor and author an independent unsigned root.
            return Err(ReplError::EpochScope);
        }
        Ok(())
    }
}

fn is_epoch_managed(doc_type: DocType) -> bool {
    matches!(
        doc_type,
        DocType::StudioIndex | DocType::StudioObject | DocType::PostReplies | DocType::DocRegistry
    )
}

fn domain_marker_key(op_id: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut key = String::with_capacity(7 + 64);
    key.push_str("_p1/op/");
    for byte in op_id {
        key.push(char::from(HEX[usize::from(byte >> 4)]));
        key.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    key
}

impl fmt::Debug for EncryptedDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncryptedDoc")
            .field("doc_type", &self.doc_type)
            .field("doc_id", &self.doc_id)
            .field("ops", &self.log.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::transaction::Transactable;
    use automerge::{ReadDoc, ROOT};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn snapshot_round_trips_a_document() {
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(1);
        let mut doc = EncryptedDoc::new(DocType::Channel, 7, &device.device_id());
        doc.edit(&device, &group, &mut rng, |d| d.put(ROOT, "k", "v1"))
            .unwrap();
        doc.edit(&device, &group, &mut rng, |d| d.put(ROOT, "k", "v2"))
            .unwrap();
        let ops = doc.op_count();

        // Snapshot, then restore from the bytes.
        let snap = doc.snapshot().unwrap();
        let mut restored = EncryptedDoc::restore(&snap).unwrap();

        // Materialized state survives…
        assert_eq!(restored.op_count(), ops);
        let v = restored.doc().get(ROOT, "k").unwrap().unwrap().0;
        assert_eq!(v.into_string().unwrap(), "v2");
        // …and the restored log still re-exports for catch-up (per-op signatures intact).
        assert_eq!(
            restored
                .export_catchup(&group, &device, &mut rng)
                .unwrap()
                .len(),
            ops
        );
        // Re-snapshotting the restored doc is stable, and garbage is rejected.
        assert!(EncryptedDoc::restore(&restored.snapshot().unwrap()).is_ok());
        assert!(EncryptedDoc::restore(b"garbage").is_err());
    }

    /// A catch-up should cost the gap, not the history. What it must not do is mistake "I cannot
    /// see that change" for "you already have everything after it": a requester whose frontier
    /// contains a change the server has never seen must still receive the server's own history.
    #[test]
    fn an_incremental_catchup_carries_the_difference_and_nothing_else() {
        let server = MlsDevice::generate().unwrap();
        let member = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&server).unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(9);

        let mut held = EncryptedDoc::new(DocType::Channel, 3, &server.device_id());
        for i in 0..6 {
            held.edit(&server, &group, &mut rng, |d| {
                d.put(ROOT, format!("k{i}"), i as u64)
            })
            .unwrap();
        }

        // A member that caught up at op 4 and then went away.
        let mut behind = EncryptedDoc::new(DocType::Channel, 3, &member.device_id());
        let full = held.export_catchup(&group, &server, &mut rng).unwrap();
        assert_eq!(full.len(), 6);
        behind.import_catchup(&full[..4], &group, &member).unwrap();
        assert_eq!(behind.op_count(), 4);

        // It comes back and names what it has: only the two it missed come down the wire.
        let gap = held
            .export_catchup_since(&behind.sync_frontier(64), &group, &server, &mut rng)
            .unwrap();
        assert_eq!(
            gap.len(),
            2,
            "only the ops behind the server's frontier and not the member's"
        );
        assert_eq!(behind.import_catchup(&gap, &group, &member).unwrap(), 2);
        assert_eq!(behind.op_count(), 6);
        for i in 0..6u64 {
            let stored = behind.doc().get(ROOT, format!("k{i}")).unwrap().unwrap().0;
            assert_eq!(stored.to_scalar().unwrap().to_u64().unwrap(), i);
        }

        // Fully caught up: the difference is empty, and asking again stays empty.
        assert!(held
            .export_catchup_since(&behind.sync_frontier(64), &group, &server, &mut rng)
            .unwrap()
            .is_empty());

        // The member writes something of its own, so its frontier now names a change the server
        // has never seen. That head can exclude nothing, and the server's own history still
        // comes back rather than being silently treated as already held.
        behind
            .edit(&member, &group, &mut rng, |d| d.put(ROOT, "mine", "yes"))
            .unwrap();
        let mut fresh = EncryptedDoc::new(DocType::Channel, 3, &server.device_id());
        fresh
            .import_catchup(
                &held.export_catchup(&group, &server, &mut rng).unwrap(),
                &group,
                &server,
            )
            .unwrap();
        let after = fresh
            .export_catchup_since(&behind.sync_frontier(64), &group, &server, &mut rng)
            .unwrap();
        assert!(
            after.is_empty(),
            "a head the server cannot see excludes nothing, and everything else is genuinely held"
        );
        let mut stranger = EncryptedDoc::new(DocType::Channel, 3, &member.device_id());
        assert_eq!(
            held.export_catchup_since(&stranger.sync_frontier(64), &group, &server, &mut rng)
                .unwrap()
                .len(),
            6,
            "a member holding nothing is told everything"
        );
    }

    /// The snapshot cache exists so an unchanged document is not re-encoded once per sent
    /// message. What it must never do is hand back bytes that predate a change: this is the
    /// durability path, and a stale snapshot is a message that was reported saved and was not.
    #[test]
    fn a_cached_snapshot_is_reused_only_while_the_document_stands_still() {
        let device = MlsDevice::generate().unwrap();
        let other = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(4);
        let mut doc = EncryptedDoc::new(DocType::Channel, 9, &device.device_id());
        doc.edit(&device, &group, &mut rng, |d| d.put(ROOT, "k", "v1"))
            .unwrap();

        let first = doc.snapshot().unwrap();
        assert_eq!(
            doc.snapshot().unwrap(),
            first,
            "an idle document re-encodes to itself"
        );
        assert!(
            doc.snapshot_cache.is_some(),
            "a small document is worth remembering"
        );

        // A local edit.
        doc.edit(&device, &group, &mut rng, |d| d.put(ROOT, "k", "v2"))
            .unwrap();
        let second = doc.snapshot().unwrap();
        assert_ne!(second, first, "a local edit invalidates the cache");
        let edited = EncryptedDoc::restore(&second).unwrap();
        let v = edited.doc().get(ROOT, "k").unwrap().unwrap().0;
        assert_eq!(v.into_string().unwrap(), "v2");

        // A peer's op arriving through the ordinary replication path.
        let mut peer = EncryptedDoc::new(DocType::Channel, 9, &other.device_id());
        peer.import_catchup(
            &doc.export_catchup(&group, &device, &mut rng).unwrap(),
            &group,
            &other,
        )
        .unwrap();
        let sealed = peer
            .edit(&other, &group, &mut rng, |d| d.put(ROOT, "peer", "here"))
            .unwrap();
        let before_ingest = doc.snapshot().unwrap();
        assert!(doc.ingest(&sealed, &group, &device).unwrap());
        let after_ingest = doc.snapshot().unwrap();
        assert_ne!(
            after_ingest, before_ingest,
            "an ingested op invalidates the cache"
        );
        let restored = EncryptedDoc::restore(&after_ingest).unwrap();
        assert!(restored.doc().get(ROOT, "peer").unwrap().is_some());
        assert_eq!(restored.op_count(), doc.op_count());

        // A document past the retention bound answers correctly and simply is not remembered.
        let mut big = EncryptedDoc::new(DocType::Channel, 10, &device.device_id());
        let payload = "x".repeat(MAX_CACHED_SNAPSHOT_BYTES + 1);
        big.edit(&device, &group, &mut rng, |d| {
            d.put(ROOT, "big", payload.as_str())
        })
        .unwrap();
        let large = big.snapshot().unwrap();
        assert!(large.len() > MAX_CACHED_SNAPSHOT_BYTES);
        assert!(
            big.snapshot_cache.is_none(),
            "the biggest documents are not duplicated"
        );
        assert_eq!(big.snapshot().unwrap(), large);
    }

    #[test]
    fn close_operations_are_topologically_canonical_not_arrival_ordered() {
        let alice = MlsDevice::generate().unwrap();
        let bob = MlsDevice::generate().unwrap();
        let logical =
            LogicalDocument::new(b"server".to_vec(), DocType::StudioObject, b"score".to_vec())
                .unwrap();
        let doc_id = crate::epoch::epoch_zero_id(logical.doc_type, &logical.logical_key);

        let make_operation = |device: &MlsDevice, nonce: [u8; 16], field: &str| {
            let domain = DomainOp {
                nonce,
                doc_type: logical.doc_type,
                logical_key: logical.logical_key.clone(),
                body: field.as_bytes().to_vec(),
            };
            let mut change_doc = AutoCommit::new();
            change_doc.set_actor(ActorId::from(device.device_id().as_bytes().to_vec()));
            change_doc.put(ROOT, field, true).unwrap();
            change_doc
                .put(
                    ROOT,
                    domain_marker_key(&domain.id(&device.device_id())),
                    1u64,
                )
                .unwrap();
            change_doc.commit();
            let change = change_doc.get_last_local_change().unwrap();
            SignedOp::sign_domain(
                device,
                logical.doc_type,
                doc_id,
                change.raw_bytes().to_vec(),
                &domain,
            )
            .unwrap()
        };
        let a = make_operation(&alice, [1; 16], "a");
        let b = make_operation(&bob, [2; 16], "b");

        let build = |arrival: [&SignedOp; 2]| {
            let mut doc = EncryptedDoc::new(logical.doc_type, doc_id, &alice.device_id());
            for operation in arrival {
                doc.doc.load_incremental(&operation.delta).unwrap();
                doc.log.push(operation.clone());
            }
            doc
        };
        let mut left = build([&a, &b]);
        let mut right = build([&b, &a]);
        let left_heads = left.heads();
        let right_heads = right.heads();
        let left_order: Vec<_> = left
            .signed_ops_for_heads(&left_heads, None)
            .unwrap()
            .into_iter()
            .map(|operation| operation.hash())
            .collect();
        let right_order: Vec<_> = right
            .signed_ops_for_heads(&right_heads, None)
            .unwrap()
            .into_iter()
            .map(|operation| operation.hash())
            .collect();
        assert_eq!(left_order, right_order);
    }

    /// What a frontier wider than its own cap costs.
    ///
    /// [`EncryptedDoc::sync_frontier`] caps the hashes a requester may name, and a serving peer
    /// subtracts only what is causally behind the hashes it was given. Past the cap the requester
    /// cannot describe everything it holds, so an honest peer answers by re-sending history the
    /// requester already has. That is safe on its own, because duplicates are dropped on arrival.
    /// It matters because it is the first half of the composition recorded in
    /// `docs/MESSAGE-FLOW.md` section 8: a catch-up round that applies nothing is exactly what the
    /// sync layer's non-progress bound counts against a source.
    ///
    /// This test establishes the replication half of that: truncation is reachable, the re-send
    /// follows from it, the re-send is pure duplicate, and it repeats identically because nothing
    /// about the exchange moved the requester's frontier.
    #[test]
    fn a_frontier_wider_than_its_cap_makes_a_peer_resend_history_already_held() {
        // Above the 64-hash cap. Each branch contributes one head; they share one ancestor, which
        // deduplicates to a single extra entry, so the cap bites at roughly this many writers.
        const BRANCHES: usize = 80;
        let author = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&author).unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(11);

        // One shared ancestor, so every branch below is concurrent with every other.
        let mut ancestor = EncryptedDoc::new(DocType::Channel, 5, &author.device_id());
        ancestor
            .edit(&author, &group, &mut rng, |d| d.put(ROOT, "seed", 0u64))
            .unwrap();
        let seed = ancestor.export_catchup(&group, &author, &mut rng).unwrap();

        // Concurrent writers. Distinct automerge actor ids are what make these changes
        // concurrent; which device signs them is irrelevant to the frontier arithmetic under
        // test here, and the sealing key is per group rather than per author.
        let mut every_op = seed.clone();
        for i in 0..BRANCHES {
            let actor = DeviceId::from_public_key_bytes(&[(i as u8).wrapping_add(1); 32]);
            let mut branch = EncryptedDoc::new(DocType::Channel, 5, &actor);
            branch.import_catchup(&seed, &group, &author).unwrap();
            branch
                .edit(&author, &group, &mut rng, |d| {
                    d.put(ROOT, format!("k{i}"), i as u64)
                })
                .unwrap();
            let ops = branch.export_catchup(&group, &author, &mut rng).unwrap();
            every_op.push(ops.last().expect("the branch's own op").clone());
        }

        // Two nodes holding byte-for-byte the same history. Nothing is missing anywhere.
        let mut requester = EncryptedDoc::new(DocType::Channel, 5, &author.device_id());
        requester
            .import_catchup(&every_op, &group, &author)
            .unwrap();
        let mut server = EncryptedDoc::new(
            DocType::Channel,
            5,
            &DeviceId::from_public_key_bytes(&[0xAA; 32]),
        );
        server.import_catchup(&every_op, &group, &author).unwrap();
        assert_eq!(requester.op_count(), BRANCHES + 1);
        assert_eq!(requester.op_count(), server.op_count());

        // The requester cannot say so. Its frontier is capped below the number of heads it holds.
        let frontier = requester.sync_frontier(64);
        assert_eq!(frontier.len(), 64, "the frontier is capped");
        assert!(
            requester.heads().len() > frontier.len(),
            "and the cap is below what this node actually holds"
        );

        // So an honest peer, subtracting only what it was told about, sends back history the
        // requester already has.
        let resent = server
            .export_catchup_since(&frontier, &group, &author, &mut rng)
            .unwrap();
        assert!(
            !resent.is_empty(),
            "the peer cannot subtract branches it was never told about"
        );
        assert_eq!(
            requester.import_catchup(&resent, &group, &author).unwrap(),
            0,
            "an entire round that moves the requester nowhere"
        );

        // And it is not a one-off. Nothing in that exchange changed either side, so the next
        // round is identical, and so is the round after it. An unbroken run of these is what the
        // sync layer's non-progress bound is counting.
        let next = requester.sync_frontier(64);
        assert_eq!(
            next, frontier,
            "the frontier did not move, so nor will this"
        );
        let again = server
            .export_catchup_since(&next, &group, &author, &mut rng)
            .unwrap();
        assert_eq!(again.len(), resent.len(), "the same answer, indefinitely");
        assert_eq!(
            requester.import_catchup(&again, &group, &author).unwrap(),
            0
        );
    }

    /// The second half of that composition, and the part that can actually starve a requester.
    ///
    /// A serving peer walks its own log in its own insertion order and the sync layer sends a
    /// size-capped **prefix** of what comes out. History the requester already holds but could not
    /// name sits at the front of that walk, because it was accepted before whatever the requester
    /// is actually missing. So the genuinely new operation is at the tail, behind a block of
    /// duplicates whose size the requester cannot influence and the server has no reason to skip.
    ///
    /// If that duplicate block does not fit inside one chunk, every answer is a prefix of the
    /// duplicates, every round applies nothing, and the new operation is never reached. The
    /// requester is not merely paying for bandwidth; it cannot converge with this peer at all,
    /// and because the duplicates come from history every member holds, the next source it tries
    /// answers exactly the same way.
    #[test]
    fn a_truncated_frontier_puts_the_missing_operation_behind_a_wall_of_duplicates() {
        const BRANCHES: usize = 80;
        let author = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&author).unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(12);

        let mut ancestor = EncryptedDoc::new(DocType::Channel, 6, &author.device_id());
        ancestor
            .edit(&author, &group, &mut rng, |d| d.put(ROOT, "seed", 0u64))
            .unwrap();
        let seed = ancestor.export_catchup(&group, &author, &mut rng).unwrap();

        let mut every_op = seed.clone();
        for i in 0..BRANCHES {
            let actor = DeviceId::from_public_key_bytes(&[(i as u8).wrapping_add(1); 32]);
            let mut branch = EncryptedDoc::new(DocType::Channel, 6, &actor);
            branch.import_catchup(&seed, &group, &author).unwrap();
            branch
                .edit(&author, &group, &mut rng, |d| {
                    d.put(ROOT, format!("k{i}"), i as u64)
                })
                .unwrap();
            let ops = branch.export_catchup(&group, &author, &mut rng).unwrap();
            every_op.push(ops.last().expect("the branch's own op").clone());
        }

        let mut requester = EncryptedDoc::new(DocType::Channel, 6, &author.device_id());
        requester
            .import_catchup(&every_op, &group, &author)
            .unwrap();
        let mut server = EncryptedDoc::new(
            DocType::Channel,
            6,
            &DeviceId::from_public_key_bytes(&[0xBB; 32]),
        );
        server.import_catchup(&every_op, &group, &author).unwrap();

        // The one thing the requester is actually missing, authored last and therefore last in
        // the server's log.
        server
            .edit(&author, &group, &mut rng, |d| {
                d.put(ROOT, "the_message_that_matters", "here")
            })
            .unwrap();

        let frontier = requester.sync_frontier(64);
        let bundle = server
            .export_catchup_since(&frontier, &group, &author, &mut rng)
            .unwrap();
        // Sixteen branches the frontier had no room to name, plus the one genuinely new
        // operation. The proportion is what matters: the duplicate block grows with the number of
        // concurrent writers, while the useful payload stays one operation.
        assert_eq!(
            bundle.len(),
            BRANCHES - 64 + 1,
            "duplicates the requester could not name, and one operation it needs"
        );

        // Every strict prefix of that answer is worthless. A chunk budget that cannot fit the
        // whole duplicate block therefore delivers nothing, however many times it is asked.
        let saved = requester.snapshot().unwrap();
        for take in 0..bundle.len() {
            let mut attempt = EncryptedDoc::restore_for_actor(&saved, &author.device_id()).unwrap();
            assert_eq!(
                attempt
                    .import_catchup(&bundle[..take], &group, &author)
                    .unwrap(),
                0,
                "a chunk holding {take} operations still carries nothing usable"
            );
            assert_eq!(
                attempt.sync_frontier(64),
                frontier,
                "and leaves the frontier exactly where it was, so the next round repeats"
            );
        }
        // Only an answer large enough to clear the entire duplicate block makes progress.
        assert_eq!(
            requester.import_catchup(&bundle, &group, &author).unwrap(),
            1,
            "the whole bundle, and only the whole bundle, converges"
        );
    }

    /// The fix for the wall above: page by position instead of recomputing the difference.
    ///
    /// Same fixture, same truncated frontier, and a budget deliberately far too small to clear the
    /// duplicate block in one answer, which is precisely the condition under which
    /// `export_catchup_since` can never converge. Because each page resumes where the last one
    /// stopped, the duplicates are consumed rather than re-offered, and the operation behind them
    /// is reached in a bounded number of rounds.
    #[test]
    fn paging_by_position_gets_past_the_wall_that_defeats_a_recomputed_difference() {
        const BRANCHES: usize = 80;
        let author = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&author).unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(13);

        let mut ancestor = EncryptedDoc::new(DocType::Channel, 7, &author.device_id());
        ancestor
            .edit(&author, &group, &mut rng, |d| d.put(ROOT, "seed", 0u64))
            .unwrap();
        let seed = ancestor.export_catchup(&group, &author, &mut rng).unwrap();

        let mut every_op = seed.clone();
        for i in 0..BRANCHES {
            let actor = DeviceId::from_public_key_bytes(&[(i as u8).wrapping_add(1); 32]);
            let mut branch = EncryptedDoc::new(DocType::Channel, 7, &actor);
            branch.import_catchup(&seed, &group, &author).unwrap();
            branch
                .edit(&author, &group, &mut rng, |d| {
                    d.put(ROOT, format!("k{i}"), i as u64)
                })
                .unwrap();
            let ops = branch.export_catchup(&group, &author, &mut rng).unwrap();
            every_op.push(ops.last().expect("the branch's own op").clone());
        }

        let mut requester = EncryptedDoc::new(DocType::Channel, 7, &author.device_id());
        requester
            .import_catchup(&every_op, &group, &author)
            .unwrap();
        let mut server = EncryptedDoc::new(
            DocType::Channel,
            7,
            &DeviceId::from_public_key_bytes(&[0xCC; 32]),
        );
        server.import_catchup(&every_op, &group, &author).unwrap();
        server
            .edit(&author, &group, &mut rng, |d| {
                d.put(ROOT, "the_message_that_matters", "here")
            })
            .unwrap();

        let frontier = requester.sync_frontier(64);
        assert_eq!(frontier.len(), 64, "the same truncated frontier as above");

        // One operation per page: `budget` is smaller than any sealed operation, and the pager
        // always takes at least one so it can never stall on an oversized entry.
        let mut position = 0usize;
        let mut applied = 0usize;
        let mut rounds = 0usize;
        let mut first_page_applied = None;
        loop {
            let (page, next) = server
                .export_catchup_page(&frontier, position, 1, &group, &author, &mut rng)
                .unwrap();
            let landed = requester.import_catchup(&page, &group, &author).unwrap();
            first_page_applied.get_or_insert(landed);
            applied += landed;
            rounds += 1;
            assert!(rounds <= BRANCHES + 2, "paging must terminate");
            match next {
                Some(resume) => {
                    assert!(resume > position, "every page consumes log");
                    position = resume;
                }
                None => break,
            }
        }

        // The first page is pure duplicate, which is exactly the round that defeats the
        // recomputing path. Here it is progress anyway, because the position moved.
        assert_eq!(
            first_page_applied,
            Some(0),
            "the wall is still in front, it is just no longer infinite"
        );
        assert_eq!(applied, 1, "and the operation behind it arrives");
        assert!(
            requester
                .doc()
                .get(ROOT, "the_message_that_matters")
                .unwrap()
                .is_some(),
            "the requester converged"
        );
        assert_eq!(
            rounds,
            BRANCHES - 64 + 1,
            "one round per operation offered: sixteen duplicates, then the one that matters, \
             whose page also reports the end because it exhausts the log"
        );
    }
}
