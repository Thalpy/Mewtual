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
use catcoms_wire::{Decoder, DocType, Encoder};

use crate::epoch::{
    Admission, AdmittedOperation, DomainOp, EpochGate, LogicalDocument, MAX_SIGNED_EPOCH_OP_BYTES,
};
use crate::op::{SealedOp, SignedOp};
use crate::ReplError;

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
}

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
        }
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
        d.finish().map_err(|_| ReplError::Malformed)?;
        Ok(Self {
            doc_type,
            doc_id,
            doc,
            log,
            applied,
            change_authors: HashMap::new(),
            authors_indexed: 0,
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
        if logical_document.doc_type != self.doc_type
            || logical_document.server_id != group.group_id()
            || domain_op.doc_type != self.doc_type
            || domain_op.logical_key != logical_document.logical_key
        {
            return Err(ReplError::EpochScope);
        }
        gate.verify_scope(logical_document, self.doc_id)?;
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
        self.check_doc(sealed.doc_type, sealed.doc_id)?;
        if logical_document.server_id != group.group_id() {
            return Err(ReplError::EpochScope);
        }
        gate.verify_scope(logical_document, self.doc_id)?;
        if sealed.epoch != group.epoch() {
            return Err(ReplError::EpochUnavailable(sealed.epoch));
        }
        let key = group.channel_secret(device, self.doc_type, self.doc_id)?;
        let op = sealed.open(&key)?;
        self.apply_domain_gated(logical_document, gate, op, validate_change)
    }

    fn apply_domain_gated<V>(
        &mut self,
        logical_document: &LogicalDocument,
        gate: &EpochGate,
        op: SignedOp,
        validate_change: V,
    ) -> Result<Admission, ReplError>
    where
        V: FnOnce(&DomainOp, &Change) -> Result<(), ReplError>,
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
}
