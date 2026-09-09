//! Studio's privately owned restart unit, using P1's existing gates, receipts and signed log.
//! It is a vault-local format, NOT a network snapshot or proof of present membership. The store
//! must journal intents, account storage and persist this unit before exposing prepared output.

use automerge::transaction::Transactable;
use automerge::{ActorId, ROOT};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};

use super::*;
use crate::epoch::{
    MAX_EPOCH_BYTES, MAX_EPOCH_GATE_BYTES, MAX_EPOCH_OPERATIONS, MAX_RECEIPT_BOOK_BYTES,
    MAX_RECEIPT_BYTES, MAX_SIGNED_EPOCH_OP_BYTES,
};
use crate::{
    epoch_zero_id, Admission, EncryptedDoc, EpochGate, EpochPhase, LocalIntent, Receipt,
    ReceiptBook, ReceiptIngest, SealedOp, SignedOp, MAX_CHECKPOINT_BYTES,
};

mod adoption;
pub mod catchup;
pub use adoption::StudioAdoptionPlan;

/// One raw seed, signed content log, gate, opening receipt and receipt book. There is no second
/// compressed Automerge save to decompress or trust. Every component also has its own bound.
pub const MAX_STUDIO_EPOCH_SNAPSHOT_BYTES: usize = MAX_CHECKPOINT_BYTES
    + MAX_EPOCH_BYTES
    + MAX_EPOCH_GATE_BYTES
    + MAX_RECEIPT_BOOK_BYTES
    + MAX_RECEIPT_BYTES
    + 4 * MAX_EPOCH_OPERATIONS
    + 1024;

/// All edits/ingest/seals require the same exclusive owner. No mutable document or gate escapes.
/// This unit can retain Closing/Fault sources, but cannot install a replacement or prune history.
pub struct StudioEpoch {
    target: StudioTarget,
    logical: LogicalDocument,
    actor: DeviceId,
    doc: EncryptedDoc,
    gate: EpochGate,
    receipts: ReceiptBook,
    opening: Option<Receipt>,
    // Version 2 is used ONLY while accepting a discovered checkpoint without its predecessor
    // closure. Ordinary restart bytes stay v1. Its receipt-book/gate validation must use the
    // existing P1 adoption mode or a crash after sealing would strand the whole source.
    adopting: bool,
    // Derived from the VERIFIED seed-only projection before any successor edits. Current
    // registers can hide an inherited replacement CID even though the retained seed needs it.
    // Recomputed on restore; never trusted from a separate persisted pin list.
    seed_blob_cids: std::collections::BTreeSet<ContentId>,
}
impl std::fmt::Debug for StudioEpoch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudioEpoch")
            .field("epoch", &self.epoch())
            .field("phase", &self.phase())
            .field("operations", &self.op_count())
            .finish_non_exhaustive()
    }
}
impl StudioEpoch {
    pub fn new(
        group: &ServerGroup,
        target: StudioTarget,
        actor: DeviceId,
    ) -> Result<Self, ReplError> {
        Self::new_scoped(&group.group_id(), target, actor, owner(group)?)
    }
    fn new_scoped(
        server: &[u8],
        target: StudioTarget,
        actor: DeviceId,
        owner: DeviceId,
    ) -> Result<Self, ReplError> {
        let logical = target.document(server)?;
        let id = epoch_zero_id(logical.doc_type, &logical.logical_key);
        Ok(Self {
            target,
            actor,
            doc: EncryptedDoc::new(logical.doc_type, id, &actor),
            gate: EpochGate::new(logical.clone(), id, 0, owner),
            logical,
            receipts: ReceiptBook::default(),
            opening: None,
            adopting: false,
            seed_blob_cids: Default::default(),
        })
    }
    /// Verify current authority and canonical typed bytes, constructing a SEPARATE successor.
    /// This grants no replacement/retirement permit; recovery-first installation is store-owned.
    pub fn from_checkpoint(
        group: &ServerGroup,
        target: StudioTarget,
        actor: DeviceId,
        receipt: Receipt,
        tenure_start: u64,
        seed: &[u8],
    ) -> Result<Self, ReplError> {
        receipt.verify_current_owner(group, tenure_start)?;
        Self::checkpoint_from_vault(
            &group.group_id(),
            target,
            actor,
            owner(group)?,
            receipt,
            seed,
        )
    }
    fn checkpoint_from_vault(
        server: &[u8],
        target: StudioTarget,
        actor: DeviceId,
        owner: DeviceId,
        receipt: Receipt,
        seed: &[u8],
    ) -> Result<Self, ReplError> {
        let logical = target.document(server)?;
        if receipt.document != logical {
            return Err(ReplError::EpochScope);
        }
        let verified = receipt.restore_verified_from_vault()?;
        let seed = match target {
            StudioTarget::Index { .. } => StudioIndexProjection::verify_checkpoint(&verified, seed),
            StudioTarget::Flipnote { channel, .. } => {
                FlipnoteFrameProjection::verify_checkpoint(&verified, channel, seed)
            }
        }?;
        let doc = EncryptedDoc::from_checkpoint(&seed, &actor)?;
        let seed_blob_cids = super::references::projection_cids(&target.read(
            &logical,
            seed.origin().epoch(),
            doc.doc(),
        )?);
        let gate = EpochGate::new(logical.clone(), doc.doc_id(), seed.origin().epoch(), owner);
        let mut receipts = ReceiptBook::default();
        receipts.ingest_verified(receipt.clone())?;
        Ok(Self {
            logical,
            target,
            actor,
            doc,
            gate,
            receipts,
            opening: Some(receipt),
            adopting: false,
            seed_blob_cids,
        })
    }
    pub fn document(&self) -> &LogicalDocument {
        &self.logical
    }
    pub fn target(&self) -> StudioTarget {
        self.target
    }
    pub fn doc_id(&self) -> u128 {
        self.doc.doc_id()
    }
    pub fn epoch(&self) -> u64 {
        self.gate.epoch()
    }
    pub fn phase(&self) -> EpochPhase {
        self.gate.phase()
    }
    /// Highest held receipt is a hint until current-owner discovery proves it. Fault refuses
    /// service rather than returning an older apparently healthy opening receipt.
    pub fn receipt_head(&self) -> Result<Option<&Receipt>, ReplError> {
        if self.phase() == EpochPhase::Fault || self.receipts.is_faulted() {
            return Err(ReplError::ReceiptConflict);
        }
        Ok(self.receipts.latest())
    }
    /// Only the installed opening's exact expected seed is served. A newer Closing receipt
    /// does not authorize generating or substituting its not-yet-installed successor bytes.
    pub fn checkpoint_bytes_by_hash(
        &mut self,
        expected_id: u128,
        expected_hash: [u8; 32],
    ) -> Result<Option<Vec<u8>>, ReplError> {
        self.receipt_head()?;
        if self.doc_id() != expected_id
            || self
                .opening
                .as_ref()
                .is_none_or(|r| r.seed_change_hash != expected_hash)
        {
            return Ok(None);
        }
        self.doc.checkpoint_bytes()
    }
    pub fn op_count(&self) -> usize {
        self.doc.op_count()
    }
    pub fn quarantined_len(&self) -> usize {
        self.gate.quarantined_len()
    }
    pub fn projection(&self) -> Result<StudioProjection, ReplError> {
        self.target
            .read(&self.logical, self.epoch(), self.doc.doc())
    }
    /// Conservative references from the whole retained source, not just its playable projection.
    /// Superseded operations must remain recoverable until history/intents are safely retired.
    pub fn blob_cids(&self) -> Result<std::collections::BTreeSet<ContentId>, ReplError> {
        let mut out = super::references::projection_cids(&self.projection()?);
        out.extend(self.seed_blob_cids.iter().copied());
        for op in self.doc.signed_log() {
            out.extend(operation_blob_cid(
                &op.parsed_domain_op()?.ok_or(ReplError::Malformed)?,
            )?);
        }
        Ok(out)
    }
    fn refresh_owner(&self, group: &ServerGroup) -> Result<(), ReplError> {
        if group.group_id() != self.logical.server_id {
            return Err(ReplError::EpochScope);
        }
        self.gate.update_owner(owner(group)?);
        Ok(())
    }
    fn held(&self, author: DeviceId, domain: &DomainOp) -> Result<Option<&SignedOp>, ReplError> {
        let id = domain.id(&author);
        for op in self
            .doc
            .signed_log()
            .iter()
            .filter(|op| op.author_device == author)
        {
            let body = op.parsed_domain_op()?.ok_or(ReplError::Malformed)?;
            if body.id(&author) == id {
                if body != *domain {
                    return Err(ReplError::IntentConflict);
                }
                return Ok(Some(op));
            }
        }
        Ok(None)
    }
    /// Read-only exact saved-operation evidence, not marker/current-value equality. This grants
    /// no edit or send authority; a caller still needs current membership and the open gate.
    /// Useful for refusing accidental create-over-existing before any new intent is journalled.
    pub fn contains_exact_operation(
        &self,
        author: DeviceId,
        domain: &DomainOp,
    ) -> Result<bool, ReplError> {
        domain.encode()?;
        if domain.doc_type != self.logical.doc_type
            || domain.logical_key != self.logical.logical_key
        {
            return Err(ReplError::EpochScope);
        }
        Ok(self.held(author, domain)?.is_some())
    }
    /// Pre-journal validation prevents invalid targets/origins or impossible projections from
    /// stranding durable intents. It authors only a detached draft; neither signs nor mutates us.
    /// A retained exact retry bypasses NEW-edit policy: later deletion/cap growth must not prevent
    /// flushing/resealing its original signed change, which cannot overwrite the later state.
    pub fn validate_local_edit(
        &self,
        device: &MlsDevice,
        group: &ServerGroup,
        domain: &DomainOp,
        ts: u64,
    ) -> Result<(), ReplError> {
        integer_bound(ts)?;
        if group.group_id() != self.logical.server_id {
            return Err(ReplError::EpochScope);
        }
        if self.actor != device.device_id()
            || group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
        {
            return Err(ReplError::EpochAuthority);
        }
        if self.phase() != EpochPhase::Open {
            return Err(ReplError::EpochClosed);
        }
        // Decode before retained-id matching; a public envelope is not intrinsically bounded.
        match self.target {
            StudioTarget::Index { .. } => {
                IndexOp::decode_domain(&self.logical, domain, &self.actor)?;
            }
            StudioTarget::Flipnote { .. } => {
                FlipnoteOp::decode_domain(&self.logical, domain)?;
            }
        }
        if self.held(self.actor, domain)?.is_some() {
            return Ok(());
        }
        let projection = self.projection()?;
        self.target.local_policy(&projection, domain, &self.actor)?;
        let prepared = match &projection {
            StudioProjection::Index(_) => {
                index::prepare(&self.logical, self.epoch(), domain, &self.actor)?
            }
            StudioProjection::Flipnote(p) => frames::prepare(p, domain, &self.actor, ts)?,
        };
        let mut draft = self
            .doc
            .doc()
            .clone()
            .with_actor(ActorId::from(self.actor.as_bytes().to_vec()));
        prepared
            .write(&mut draft)
            .map_err(crate::checkpoint::am_error)?;
        draft
            .put(
                ROOT,
                format!("_p1/op/{}", hex(&domain.id(&self.actor))),
                1u64,
            )
            .map_err(crate::checkpoint::am_error)?;
        draft.commit();
        let change = draft.get_last_local_change().ok_or(ReplError::Malformed)?;
        self.target
            .validate(&self.logical, self.epoch(), domain, &change, self.doc.doc())?;
        let mut operations = recovery::current_operations(&self.doc)?;
        operations.insert(
            domain.id(&self.actor),
            LocalIntent {
                author: self.actor,
                operation: domain.clone(),
            },
        );
        recovery::preflight(
            self.target.read(&self.logical, self.epoch(), &draft)?,
            &operations,
        )
    }
    /// Caller must journal the intent first and persist the whole unit before publishing output.
    /// Retry compares the complete envelope, not its body-excluding id, and never authors again.
    pub fn edit_or_reseal(
        &mut self,
        device: &MlsDevice,
        group: &ServerGroup,
        rng: &mut impl CryptoRngCore,
        domain: &DomainOp,
        ts: u64,
    ) -> Result<SealedOp, ReplError> {
        self.validate_local_edit(device, group, domain, ts)?;
        self.refresh_owner(group)?;
        if let Some(held) = self.held(self.actor, domain)? {
            return SealedOp::seal(held, group, device, rng);
        }
        self.target
            .edit(&mut self.doc, &self.gate, device, group, rng, domain, ts)
    }
    pub fn ingest(
        &mut self,
        sealed: &SealedOp,
        group: &ServerGroup,
        device: &MlsDevice,
    ) -> Result<Admission, ReplError> {
        if device.device_id() != self.actor {
            return Err(ReplError::EpochAuthority);
        }
        self.refresh_owner(group)?;
        self.target
            .ingest(&mut self.doc, &self.gate, sealed, group, device)
    }
    /// Seals the entire held source under the same exclusive borrow as edit/ingest. No pruning.
    /// Nonadjacent discovery uses explicit adoption mode; durable repair remains a separate API.
    pub fn seal(
        &mut self,
        receipt: Receipt,
        group: &ServerGroup,
        tenure_start: u64,
    ) -> Result<ReceiptIngest, ReplError> {
        self.refresh_owner(group)?;
        if self.adopting {
            return self.begin_checkpoint_adoption(receipt, group, tenure_start);
        }
        if let Some(opening) = &self.opening {
            if receipt.closed_epoch == opening.closed_epoch {
                return self.receipts.check_opening_receipt(
                    receipt,
                    opening,
                    group,
                    tenure_start,
                    &self.gate,
                );
            }
        }
        self.receipts
            .ingest_and_seal(receipt, group, tenure_start, &self.gate)
            .map(|(outcome, _)| outcome)
    }
    /// Bounded plaintext for authenticated vault storage only. Channel is part of the checked
    /// restart identity even though an object's globally unique logical key is just its id.
    pub fn snapshot(&mut self) -> Result<Vec<u8>, ReplError> {
        let mut e = Encoder::new();
        e.put_u8(if self.adopting { 2 } else { 1 });
        e.put_bytes(&self.target.channel())
            .map_err(|_| ReplError::EpochBound)?;
        for bytes in [
            self.opening
                .as_ref()
                .map(Receipt::encode)
                .unwrap_or_default(),
            self.doc.checkpoint_bytes()?.unwrap_or_default(),
            self.receipt_book_bytes()?,
            self.gate.encode()?,
        ] {
            e.put_bytes(&bytes).map_err(|_| ReplError::EpochBound)?;
        }
        e.put_u32(
            self.op_count()
                .try_into()
                .map_err(|_| ReplError::EpochBound)?,
        );
        for op in self.doc.signed_log() {
            e.put_bytes(&op.encode())
                .map_err(|_| ReplError::EpochBound)?;
        }
        let bytes = e.finish();
        if bytes.len() > MAX_STUDIO_EPOCH_SNAPSHOT_BYTES {
            return Err(ReplError::EpochBound);
        }
        Ok(bytes)
    }
    /// Only locally authenticated vault bytes qualify. Historical authors/owners may be removed;
    /// signatures and causal mutations still verify, while new edits use current membership.
    pub fn restore(
        bytes: &[u8],
        group: &ServerGroup,
        target: StudioTarget,
        actor: DeviceId,
    ) -> Result<Self, ReplError> {
        Self::restore_scoped(bytes, &group.group_id(), target, actor, owner(group)?)
    }
    /// Detached vault verification with captured public context, never network bytes. The
    /// caller must recheck current membership/owner/MLS and exact authenticated vault bytes
    /// before attaching this result. Edits still require the ordinary live gate and signer.
    pub fn prepare_vault_source(
        bytes: &[u8],
        server: &[u8],
        target: StudioTarget,
        actor: DeviceId,
        owner: DeviceId,
    ) -> Result<Self, ReplError> {
        Self::restore_scoped(bytes, server, target, actor, owner)
    }
    /// Inventory-only validation after removal of a server/author; returns no writable capability.
    pub fn validate_vault_snapshot(
        bytes: &[u8],
        server: &[u8],
        target: StudioTarget,
    ) -> Result<usize, ReplError> {
        let inert = DeviceId::from_bytes([0; 32]);
        Self::restore_scoped(bytes, server, target, inert, inert)?.storage_protocol_bytes()
    }
    /// Same full vault-only verification as inventory, also returning conservative references.
    /// No current membership or writable capability is inferred from historical authors.
    pub fn inspect_vault_references(
        bytes: &[u8],
        server: &[u8],
        target: StudioTarget,
    ) -> Result<(usize, std::collections::BTreeSet<ContentId>), ReplError> {
        let inert = DeviceId::from_bytes([0; 32]);
        let unit = Self::restore_scoped(bytes, server, target, inert, inert)?;
        Ok((unit.storage_protocol_bytes()?, unit.blob_cids()?))
    }
    /// Only receipt bytes and the gate's closing hash consume protocol headroom. User content,
    /// seeds, admission metadata and quarantine hashes cannot spend the settlement allowance.
    pub fn storage_protocol_bytes(&self) -> Result<usize, ReplError> {
        let book = self
            .receipt_book_bytes()?
            .len()
            .checked_sub(ReceiptBook::default().encode()?.len())
            .ok_or(ReplError::Malformed)?;
        let opening = self.opening.as_ref().map_or(0, |r| r.encode().len());
        let gate = if matches!(self.phase(), EpochPhase::Closing | EpochPhase::Settled) {
            36
        } else {
            0
        };
        Ok(book + opening + gate)
    }
    fn restore_scoped(
        bytes: &[u8],
        server: &[u8],
        target: StudioTarget,
        actor: DeviceId,
        owner: DeviceId,
    ) -> Result<Self, ReplError> {
        if bytes.len() > MAX_STUDIO_EPOCH_SNAPSHOT_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        let adopting = match d.get_u8().map_err(|_| ReplError::Malformed)? {
            1 => false,
            2 => true,
            _ => return Err(ReplError::Malformed),
        };
        if d.get_bytes().map_err(|_| ReplError::Malformed)? != target.channel() {
            return Err(ReplError::EpochScope);
        }
        let opening = field(&mut d, MAX_RECEIPT_BYTES)?;
        let seed = field(&mut d, MAX_CHECKPOINT_BYTES)?;
        let book = field(&mut d, MAX_RECEIPT_BOOK_BYTES)?;
        let receipts = if adopting {
            ReceiptBook::decode_adoption(book)?
        } else {
            ReceiptBook::decode(book)?
        };
        let gate = EpochGate::decode(field(&mut d, MAX_EPOCH_GATE_BYTES)?)?;
        let count = d.get_u32().map_err(|_| ReplError::Malformed)? as usize;
        if count > MAX_EPOCH_OPERATIONS {
            return Err(ReplError::EpochBound);
        }
        let mut total = 0usize;
        let mut operations = Vec::new();
        for _ in 0..count {
            let bytes = field(&mut d, MAX_SIGNED_EPOCH_OP_BYTES)?;
            total = total.saturating_add(bytes.len());
            if total > MAX_EPOCH_BYTES {
                return Err(ReplError::EpochBound);
            }
            operations.push(SignedOp::decode(bytes)?);
        }
        d.finish().map_err(|_| ReplError::Malformed)?;
        let mut result = if opening.is_empty() {
            if !seed.is_empty() {
                return Err(ReplError::Malformed);
            }
            Self::new_scoped(server, target, actor, owner)?
        } else {
            Self::checkpoint_from_vault(
                server,
                target,
                actor,
                owner,
                Receipt::decode(opening)?,
                seed,
            )?
        };
        gate.verify_scope(&result.logical, result.doc_id())?;
        if gate.epoch() != result.epoch() {
            return Err(ReplError::EpochScope);
        }
        let epoch = gate.epoch();
        let metadata = result.doc.restore_domain_log(
            &result.logical,
            operations,
            |domain, change, before, _| {
                target.validate(&result.logical, epoch, domain, change, before)
            },
        )?;
        if adopting {
            gate.verify_adoption_restart(&metadata, &receipts, result.opening.as_ref())?;
        } else {
            gate.verify_restart(&metadata, &receipts, result.opening.as_ref())?;
        }
        recovery::preflight(
            result.projection()?,
            &recovery::current_operations(&result.doc)?,
        )?;
        result.gate = gate;
        result.receipts = receipts;
        result.adopting = adopting;
        result.gate.update_owner(owner);
        Ok(result)
    }
    fn receipt_book_bytes(&self) -> Result<Vec<u8>, ReplError> {
        if self.adopting {
            self.receipts.encode_adoption()
        } else {
            self.receipts.encode()
        }
    }
}
fn owner(group: &ServerGroup) -> Result<DeviceId, ReplError> {
    group
        .designated_committer()
        .ok_or(ReplError::EpochAuthority)
}
fn field<'a>(d: &mut Decoder<'a>, max: usize) -> Result<&'a [u8], ReplError> {
    let bytes = d.get_bytes().map_err(|_| ReplError::Malformed)?;
    if bytes.len() > max {
        return Err(ReplError::EpochBound);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
