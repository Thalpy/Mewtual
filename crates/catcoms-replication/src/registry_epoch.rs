//! One registry epoch's admission, document and receipt state, serialized as one restart unit.
//!
//! This is an in-memory coordinator, not a disk transaction or a settlement worker. The caller
//! must vault-seal and atomically persist its snapshot before publishing edits or acknowledging
//! receipts. Closing retains the entire accepted log. Recovery persistence, successor selection,
//! pruning and transport discovery are deliberately not exposed here.

use catcoms_crypto::DeviceId;
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, DocType, Encoder};

use crate::epoch::{
    MAX_EPOCH_BYTES, MAX_EPOCH_GATE_BYTES, MAX_EPOCH_OPERATIONS, MAX_RECEIPT_BOOK_BYTES,
    MAX_RECEIPT_BYTES, MAX_SIGNED_EPOCH_OP_BYTES,
};
use crate::registry::{
    edit_registry, ingest_registry, preflight, registry_document, validate_domain,
    validate_registry_change, RegistryProjection, MAX_REGISTRY_EPOCH,
};
use crate::{
    epoch_zero_id, Admission, DomainOp, EncryptedDoc, EpochGate, EpochPhase, LogicalDocument,
    Receipt, ReceiptBook, ReceiptIngest, ReplError, SealedOp, SignedOp, MAX_CHECKPOINT_BYTES,
};

pub mod catchup;
mod settlement;
pub use settlement::RegistrySettlementPlan;

/// Raw seed + signed content + gate + receipts, including bounded length framing. There is no
/// second, potentially compressed Automerge save to trust or decompress during restore.
pub const MAX_REGISTRY_EPOCH_SNAPSHOT_BYTES: usize = MAX_CHECKPOINT_BYTES
    + MAX_EPOCH_BYTES
    + MAX_EPOCH_GATE_BYTES
    + MAX_RECEIPT_BOOK_BYTES
    + MAX_RECEIPT_BYTES
    + 4 * MAX_EPOCH_OPERATIONS
    + 1024;

/// Private ownership prevents callers from mutating a document without its gate, or sealing a
/// gate without its receipt book. All lifecycle operations require the same exclusive borrow.
#[derive(Debug)]
pub struct RegistryEpoch {
    logical: LogicalDocument,
    bucket: u8,
    actor: DeviceId,
    doc: EncryptedDoc,
    gate: EpochGate,
    receipts: ReceiptBook,
    opening: Option<Receipt>,
}

impl RegistryEpoch {
    /// Start the deterministic epoch-zero bucket. An empty bucket needs no owner receipt.
    pub fn new(group: &ServerGroup, bucket: u8, actor: DeviceId) -> Result<Self, ReplError> {
        let owner = group
            .designated_committer()
            .ok_or(ReplError::EpochAuthority)?;
        Self::new_scoped(&group.group_id(), bucket, actor, owner)
    }

    fn new_scoped(
        server: &[u8],
        bucket: u8,
        actor: DeviceId,
        owner: DeviceId,
    ) -> Result<Self, ReplError> {
        let logical = registry_document(server, bucket)?;
        let id = epoch_zero_id(DocType::DocRegistry, &logical.logical_key);
        Ok(Self {
            doc: EncryptedDoc::new(DocType::DocRegistry, id, &actor),
            gate: EpochGate::new(logical.clone(), id, 0, owner),
            logical,
            bucket,
            actor,
            receipts: ReceiptBook::default(),
            opening: None,
        })
    }

    /// Construct a separately owned successor from a current-owner receipt and its exact seed.
    /// This does NOT replace, settle or discard a predecessor. The caller must first satisfy
    /// recovery/durable-install ordering before selecting this as its current epoch.
    pub fn from_checkpoint(
        group: &ServerGroup,
        bucket: u8,
        actor: DeviceId,
        receipt: Receipt,
        expected_tenure_start: u64,
        seed: &[u8],
    ) -> Result<Self, ReplError> {
        receipt.verify_current_owner(group, expected_tenure_start)?;
        let owner = group
            .designated_committer()
            .ok_or(ReplError::EpochAuthority)?;
        Self::checkpoint_from_vault(&group.group_id(), bucket, actor, owner, receipt, seed)
    }

    // Only the public constructor (fresh authority) and authenticated-vault restore may enter.
    fn checkpoint_from_vault(
        server: &[u8],
        bucket: u8,
        actor: DeviceId,
        owner: DeviceId,
        receipt: Receipt,
        seed: &[u8],
    ) -> Result<Self, ReplError> {
        let logical = registry_document(server, bucket)?;
        if receipt.document != logical {
            return Err(ReplError::EpochScope);
        }
        let verified = receipt.restore_verified_from_vault()?;
        let checkpoint = RegistryProjection::verify_checkpoint(&verified, bucket, seed)?;
        let doc = EncryptedDoc::from_checkpoint(&checkpoint, &actor)?;
        let gate = EpochGate::new(
            logical.clone(),
            doc.doc_id(),
            checkpoint.origin().epoch(),
            owner,
        );
        let mut receipts = ReceiptBook::default();
        receipts.ingest_verified(receipt.clone())?;
        Ok(Self {
            logical,
            bucket,
            actor,
            doc,
            gate,
            receipts,
            opening: Some(receipt),
        })
    }

    /// Concrete document id for routing, never a mutable document handle.
    pub fn doc_id(&self) -> u128 {
        self.doc.doc_id()
    }
    /// Epoch number of the retained log (including while Closing or Fault).
    pub fn epoch(&self) -> u64 {
        self.gate.epoch()
    }
    /// Admission state; Closing is not a claim that recovery or disk settlement has finished.
    pub fn phase(&self) -> EpochPhase {
        self.gate.phase()
    }
    /// Historical receipt selected in this saved unit, not current-owner authority or proof
    /// that its seed is available. Fault must never be presented as an absent/ordinary head.
    pub fn receipt_head(&self) -> Result<Option<&Receipt>, ReplError> {
        if self.phase() == EpochPhase::Fault || self.receipts.is_faulted() {
            return Err(ReplError::ReceiptConflict);
        }
        Ok(self.receipts.latest())
    }
    /// Read only the installed opening seed, never the latest receipt's prospective successor.
    /// Closing can hold a newer head whose seed is not installed yet; that exact request is
    /// unavailable rather than permission to generate/substitute a root. Fault serves nothing.
    pub fn checkpoint_bytes_by_hash(
        &mut self,
        expected_doc_id: u128,
        expected_hash: [u8; 32],
    ) -> Result<Option<Vec<u8>>, ReplError> {
        self.receipt_head()?;
        if self.doc_id() != expected_doc_id
            || self
                .opening
                .as_ref()
                .is_none_or(|r| r.seed_change_hash != expected_hash)
        {
            return Ok(None);
        }
        self.doc.checkpoint_bytes()
    }
    /// Complete accepted-log count. Receipt admission does not reduce it.
    pub fn op_count(&self) -> usize {
        self.doc.op_count()
    }
    /// Distinct late hashes retained without accepting their bodies.
    pub fn quarantined_len(&self) -> usize {
        self.gate.quarantined_len()
    }
    /// Materialize a detached value for callers; no mutable replication state escapes.
    pub fn projection(&self) -> Result<RegistryProjection, ReplError> {
        RegistryProjection::read(&self.logical, self.bucket, self.epoch(), self.doc.doc())
    }

    fn refresh_owner(&self, group: &ServerGroup) -> Result<(), ReplError> {
        if group.group_id() != self.logical.server_id {
            return Err(ReplError::EpochScope);
        }
        self.gate.update_owner(
            group
                .designated_committer()
                .ok_or(ReplError::EpochAuthority)?,
        );
        Ok(())
    }

    /// Apply an already durably journaled intent. The returned ciphertext is pending publication
    /// until this entire restart unit is committed by the storage owner.
    pub fn edit(
        &mut self,
        device: &MlsDevice,
        group: &ServerGroup,
        rng: &mut impl CryptoRngCore,
        domain: &DomainOp,
    ) -> Result<SealedOp, ReplError> {
        if device.device_id() != self.actor {
            return Err(ReplError::EpochAuthority);
        }
        self.refresh_owner(group)?;
        edit_registry(
            &mut self.doc,
            &self.gate,
            self.bucket,
            device,
            group,
            rng,
            domain,
        )
    }

    /// Check canonical semantics, scope, local authority, lifecycle and retained-id conflicts
    /// BEFORE journaling an intent. This is not the prospective projection/storage preflight;
    /// those still run on edit. Failure here must not strand an irreconcilable durable intent.
    pub fn validate_local_edit(
        &self,
        device: &MlsDevice,
        group: &ServerGroup,
        domain: &DomainOp,
    ) -> Result<(), ReplError> {
        validate_domain(&self.logical, self.bucket, domain)?;
        if group.group_id() != self.logical.server_id {
            return Err(ReplError::EpochScope);
        }
        if device.device_id() != self.actor
            || group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
        {
            return Err(ReplError::EpochAuthority);
        }
        if self.phase() != EpochPhase::Open {
            return Err(ReplError::EpochClosed);
        }
        if self.epoch() >= MAX_REGISTRY_EPOCH {
            return Err(ReplError::EpochBound);
        }
        self.held_local_operation(device, domain)?;
        Ok(())
    }

    // This scan is bounded by the epoch log caps. Markers alone cannot identify the bytes to
    // republish, and the id omits the body: compare the whole canonical envelope, not just its id.
    fn held_local_operation(
        &self,
        device: &MlsDevice,
        domain: &DomainOp,
    ) -> Result<Option<&SignedOp>, ReplError> {
        let author = device.device_id();
        let id = domain.id(&author);
        for op in self
            .doc
            .signed_log()
            .iter()
            .filter(|op| op.author_device == author)
        {
            let held = op.parsed_domain_op()?.ok_or(ReplError::Malformed)?;
            if held.id(&author) == id {
                if held != *domain {
                    return Err(ReplError::IntentConflict);
                }
                return Ok(Some(op));
            }
        }
        Ok(None)
    }

    /// Whether this Open unit retains the exact authenticated local change, not merely a marker.
    /// Replay may reseal such a change without reapplying its effect over newer state. Scope,
    /// membership, lifecycle and full-envelope equality are checked just as for local editing;
    /// the result is not proof of disk durability or a continuing network-send permit.
    pub fn retains_local_operation(
        &self,
        device: &MlsDevice,
        group: &ServerGroup,
        domain: &DomainOp,
    ) -> Result<bool, ReplError> {
        self.validate_local_edit(device, group, domain)?;
        Ok(self.held_local_operation(device, domain)?.is_some())
    }

    /// Apply a durably journaled intent or reseal its EXACT retained signed change on retry.
    /// Never reauthor a saved operation against newer heads. Current membership and Open are
    /// required even for a retry. This only prepares ciphertext: the caller must persist/flush
    /// the whole unit before exposing it, and recheck session/group/epoch at actual network send.
    pub fn edit_or_reseal(
        &mut self,
        device: &MlsDevice,
        group: &ServerGroup,
        rng: &mut impl CryptoRngCore,
        domain: &DomainOp,
    ) -> Result<SealedOp, ReplError> {
        self.validate_local_edit(device, group, domain)?;
        self.refresh_owner(group)?;
        if let Some(held) = self.held_local_operation(device, domain)? {
            return SealedOp::seal(held, group, device, rng);
        }
        self.edit(device, group, rng, domain)
    }

    /// Authenticate, type-check and admit one network operation through the same epoch gate.
    pub fn ingest(
        &mut self,
        sealed: &SealedOp,
        group: &ServerGroup,
        device: &MlsDevice,
    ) -> Result<Admission, ReplError> {
        self.refresh_owner(group)?;
        ingest_registry(
            &mut self.doc,
            &self.gate,
            self.bucket,
            sealed,
            group,
            device,
        )
    }

    /// Verify owner/tenure and seal admission atomically. All source content remains available;
    /// this API cannot finish settlement or bypass a failed recovery write.
    /// A different new-tenure receipt while already Closing returns `ReceiptConflict` unchanged:
    /// applying that adoption/rewind requires the future recovery-first settlement worker.
    pub fn seal(
        &mut self,
        receipt: Receipt,
        group: &ServerGroup,
        expected_tenure_start: u64,
    ) -> Result<ReceiptIngest, ReplError> {
        self.refresh_owner(group)?;
        if let Some(opening) = &self.opening {
            if receipt.closed_epoch == opening.closed_epoch {
                return self.receipts.check_opening_receipt(
                    receipt,
                    opening,
                    group,
                    expected_tenure_start,
                    &self.gate,
                );
            }
        }
        self.receipts
            .ingest_and_seal(receipt, group, expected_tenure_start, &self.gate)
            .map(|(outcome, _)| outcome)
    }

    /// Encode one plaintext restart unit for authenticated, atomic vault persistence. It is NOT
    /// a wire format: receipt history and past membership admission are trusted only locally.
    pub fn snapshot(&mut self) -> Result<Vec<u8>, ReplError> {
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_u8(self.bucket);
        for bytes in [
            self.opening
                .as_ref()
                .map(Receipt::encode)
                .unwrap_or_default(),
            self.doc.checkpoint_bytes()?.unwrap_or_default(),
            self.receipts.encode()?,
            self.gate.encode()?,
        ] {
            e.put_bytes(&bytes).map_err(|_| ReplError::EpochBound)?;
        }
        e.put_u32(
            self.doc
                .op_count()
                .try_into()
                .map_err(|_| ReplError::EpochBound)?,
        );
        for op in self.doc.signed_log() {
            e.put_bytes(&op.encode())
                .map_err(|_| ReplError::EpochBound)?;
        }
        let bytes = e.finish();
        if bytes.len() > MAX_REGISTRY_EPOCH_SNAPSHOT_BYTES {
            return Err(ReplError::EpochBound);
        }
        Ok(bytes)
    }

    /// Restore ONLY bytes authenticated by this device's vault, with externally expected server
    /// and bucket scope. Rebuild from the authorized raw seed and bounded signed log. No current
    /// roster check is applied to historical edits/receipts; new admission uses today's owner.
    pub fn restore(
        bytes: &[u8],
        group: &ServerGroup,
        bucket: u8,
        actor: DeviceId,
    ) -> Result<Self, ReplError> {
        let owner = group
            .designated_committer()
            .ok_or(ReplError::EpochAuthority)?;
        Self::restore_scoped(bytes, &group.group_id(), bucket, actor, owner)
    }

    /// Validate a locally authenticated vault snapshot for storage inventory, even after the
    /// server or its former owner has left. Returns no editable object, receipt capability or
    /// publication permission. Like restore, this MUST NOT authenticate network history.
    /// The sole returned number is the recomputed receipt-only protocol footprint, not a
    /// peer-supplied claim; subtract it from the actual encoded record length for content usage.
    pub fn validate_vault_snapshot(
        bytes: &[u8],
        server: &[u8],
        bucket: u8,
    ) -> Result<usize, ReplError> {
        // Inventory authors nothing. Fixed identities avoid ambient randomness and are never
        // exposed: the reconstructed graph exists only to run the same full consistency checks.
        let inert = DeviceId::from_bytes([0; 32]);
        Self::restore_scoped(bytes, server, bucket, inert, inert)?.storage_protocol_bytes()
    }

    /// Actual snapshot bytes introduced by receipts, charged to the protocol allowance. All
    /// peer-writable registry content, seeds, admission metadata and quarantine hashes charge
    /// ordinary content instead. This keeps user writes from consuming receipt headroom, while
    /// an owner seal at the content ceiling can still add its bounded protocol state.
    pub fn storage_protocol_bytes(&self) -> Result<usize, ReplError> {
        let book_growth = self
            .receipts
            .encode()?
            .len()
            .checked_sub(ReceiptBook::default().encode()?.len())
            .ok_or(ReplError::Malformed)?;
        let opening = self
            .opening
            .as_ref()
            .map_or(0, |receipt| receipt.encode().len());
        // EpochGate v1 adds a length-framed 32-byte hash after its existing option tag only in
        // Closing/Settled. Pin this layout-dependent accounting against snapshot deltas in tests.
        let gate_hash = if matches!(self.phase(), EpochPhase::Closing | EpochPhase::Settled) {
            36
        } else {
            0
        };
        Ok(book_growth + opening + gate_hash)
    }

    fn restore_scoped(
        bytes: &[u8],
        server: &[u8],
        bucket: u8,
        actor: DeviceId,
        owner: DeviceId,
    ) -> Result<Self, ReplError> {
        if bytes.len() > MAX_REGISTRY_EPOCH_SNAPSHOT_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        if d.get_u8().map_err(|_| ReplError::Malformed)? != bucket {
            return Err(ReplError::EpochScope);
        }
        let opening = field(&mut d, MAX_RECEIPT_BYTES)?;
        let seed = field(&mut d, MAX_CHECKPOINT_BYTES)?;
        let receipts = ReceiptBook::decode(field(&mut d, MAX_RECEIPT_BOOK_BYTES)?)?;
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
            Self::new_scoped(server, bucket, actor, owner)?
        } else {
            Self::checkpoint_from_vault(
                server,
                bucket,
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
            |domain, change, before| {
                validate_registry_change(&result.logical, bucket, epoch, domain, change, before)
            },
        )?;
        gate.verify_restart(&metadata, &receipts, result.opening.as_ref())?;
        // Prefixes were preflighted before their original vault admission. On restart validate
        // every signed change's semantics but encode the full prospective seed only once.
        if epoch < crate::registry::MAX_REGISTRY_EPOCH {
            preflight(&result.logical, bucket, epoch, result.doc.doc())?;
        } else {
            result.projection()?; // The terminal registry epoch remains readable, not editable.
        }
        result.gate = gate;
        result.receipts = receipts;
        result.gate.update_owner(owner);
        Ok(result)
    }
}

fn field<'a>(d: &mut Decoder<'a>, cap: usize) -> Result<&'a [u8], ReplError> {
    let bytes = d.get_bytes().map_err(|_| ReplError::Malformed)?;
    if bytes.len() > cap {
        return Err(ReplError::EpochBound);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{PointerKey, RegistryOp};
    use crate::{AdmittedOperation, InheritedCheckpoint};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn registry_vault_inspection_reuses_scope_validation_and_exact_receipt_accounting() {
        let mut f = Fixture::new();
        let mut unit = f.empty();
        f.edit(&mut unit, 1);
        let original = unit.snapshot().unwrap();
        assert_eq!(
            RegistryEpoch::validate_vault_snapshot(&original, &f.group.group_id(), f.key.bucket())
                .unwrap(),
            0
        );
        assert!(
            RegistryEpoch::validate_vault_snapshot(&original, b"wrong-group", f.key.bucket())
                .is_err()
        );
        assert!(RegistryEpoch::validate_vault_snapshot(
            &original,
            &f.group.group_id(),
            f.key.bucket().wrapping_add(1)
        )
        .is_err());
        for close in [7, 8] {
            unit.seal(f.receipt(&unit, close), &f.group, 0).unwrap();
            let bytes = unit.snapshot().unwrap();
            let protocol =
                RegistryEpoch::validate_vault_snapshot(&bytes, &f.group.group_id(), f.key.bucket())
                    .unwrap();
            assert_eq!(
                bytes.len() - protocol,
                original.len(),
                "seal/fault growth is protocol only"
            );
            assert_eq!(protocol, unit.storage_protocol_bytes().unwrap());
            let mut malformed = bytes.clone();
            malformed.push(0);
            assert!(RegistryEpoch::validate_vault_snapshot(
                &malformed,
                &f.group.group_id(),
                f.key.bucket()
            )
            .is_err());
        }
    }

    struct Fixture {
        owner: MlsDevice,
        group: ServerGroup,
        rng: ChaCha20Rng,
        key: PointerKey,
    }

    #[test]
    fn registry_seed_serving_uses_opening_origin_through_closing_restart_and_fault() {
        let mut f = Fixture::new();
        let mut epoch = f.empty();
        f.edit(&mut epoch, 1);
        let receipt = f.receipt(&epoch, 7);
        let seed = epoch.projection().unwrap().checkpoint([7; 32]).unwrap();
        assert!(epoch
            .checkpoint_bytes_by_hash(seed.origin().doc_id(), seed.change_hash())
            .unwrap()
            .is_none());
        let mut successor = RegistryEpoch::from_checkpoint(
            &f.group,
            f.key.bucket(),
            f.owner.device_id(),
            receipt,
            0,
            seed.bytes(),
        )
        .unwrap();
        let id = successor.doc_id();
        assert_eq!(
            successor
                .checkpoint_bytes_by_hash(id, seed.change_hash())
                .unwrap()
                .unwrap(),
            seed.bytes()
        );
        assert!(successor
            .checkpoint_bytes_by_hash(id ^ 1, seed.change_hash())
            .unwrap()
            .is_none());
        assert!(successor
            .checkpoint_bytes_by_hash(id, [0; 32])
            .unwrap()
            .is_none());
        let next = f.receipt(&successor, 8);
        successor.seal(next, &f.group, 0).unwrap();
        let next_seed = successor.projection().unwrap().checkpoint([8; 32]).unwrap();
        assert!(successor
            .checkpoint_bytes_by_hash(next_seed.origin().doc_id(), next_seed.change_hash())
            .unwrap()
            .is_none());
        let mut restored = f.restore(&successor.snapshot().unwrap()).unwrap();
        assert_eq!(restored.phase(), EpochPhase::Closing);
        assert_eq!(
            restored
                .checkpoint_bytes_by_hash(id, seed.change_hash())
                .unwrap()
                .unwrap(),
            seed.bytes()
        );
        restored.seal(f.receipt(&restored, 9), &f.group, 0).unwrap();
        assert_eq!(restored.phase(), EpochPhase::Fault);
        assert!(restored
            .checkpoint_bytes_by_hash(id, seed.change_hash())
            .is_err());
    }
    impl Fixture {
        fn new() -> Self {
            let owner = MlsDevice::generate().unwrap();
            let group = ServerGroup::create(&owner).unwrap();
            Self {
                owner,
                group,
                rng: ChaCha20Rng::seed_from_u64(61),
                key: PointerKey::new(DocType::StudioObject, b"a".to_vec()).unwrap(),
            }
        }
        fn empty(&self) -> RegistryEpoch {
            RegistryEpoch::new(&self.group, self.key.bucket(), self.owner.device_id()).unwrap()
        }
        fn domain(&self, n: u8) -> DomainOp {
            RegistryOp::Put {
                key: self.key.clone(),
                epoch: u64::from(n),
            }
            .domain_op(&self.group.group_id(), [n; 16])
            .unwrap()
        }
        fn edit(&mut self, epoch: &mut RegistryEpoch, n: u8) -> SealedOp {
            let domain = self.domain(n);
            epoch
                .edit(&self.owner, &self.group, &mut self.rng, &domain)
                .unwrap()
        }
        fn receipt(&self, epoch: &RegistryEpoch, close: u8) -> Receipt {
            let seed = epoch.projection().unwrap().checkpoint([close; 32]).unwrap();
            Receipt::sign(
                epoch.logical.clone(),
                epoch.epoch(),
                [close; 32],
                seed.change_hash(),
                0,
                InheritedCheckpoint::EpochZero,
                &self.owner,
            )
            .unwrap()
        }
        fn restore(&self, bytes: &[u8]) -> Result<RegistryEpoch, ReplError> {
            RegistryEpoch::restore(
                bytes,
                &self.group,
                self.key.bucket(),
                self.owner.device_id(),
            )
        }
    }

    #[test]
    fn registry_replay_presence_requires_full_signed_envelope_not_marker() {
        use automerge::transaction::Transactable;
        let mut f = Fixture::new();
        let mut epoch = f.empty();
        let first = f.domain(1);
        assert!(!epoch
            .retains_local_operation(&f.owner, &f.group, &first)
            .unwrap());
        // Deliberately bypass the checked RegistryEpoch restart constructor to test that even
        // a forged marker in a legacy generic document is not treated as retained signed work.
        let mut graph = epoch.doc.doc().clone();
        let id: String = first
            .id(&f.owner.device_id())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        graph
            .put(automerge::ROOT, format!("_p1/op/{id}"), 1u64)
            .unwrap();
        let mut e = Encoder::new();
        e.put_u16(DocType::DocRegistry.tag());
        e.put_u128(epoch.doc_id());
        e.put_bytes(&graph.save()).unwrap();
        e.put_u32(0); // no authenticated log
        epoch.doc = EncryptedDoc::restore_for_actor(&e.finish(), &f.owner.device_id()).unwrap();
        assert!(!epoch
            .retains_local_operation(&f.owner, &f.group, &first)
            .unwrap());
        let mut epoch = f.empty();
        f.edit(&mut epoch, 1);
        assert!(epoch
            .retains_local_operation(&f.owner, &f.group, &first)
            .unwrap());
        let mut different = f.domain(2);
        different.nonce = first.nonce;
        assert!(matches!(
            epoch.retains_local_operation(&f.owner, &f.group, &different),
            Err(ReplError::IntentConflict)
        ));
        epoch.seal(f.receipt(&epoch, 7), &f.group, 0).unwrap();
        assert!(matches!(
            epoch.retains_local_operation(&f.owner, &f.group, &first),
            Err(ReplError::EpochClosed)
        ));
    }

    #[test]
    fn registry_local_retry_keeps_exact_signed_change_after_restart_and_new_heads() {
        let mut f = Fixture::new();
        let mut epoch = f.empty();
        let first = f.domain(1);
        epoch
            .edit_or_reseal(&f.owner, &f.group, &mut f.rng, &first)
            .unwrap();
        let original = epoch.doc.signed_log()[0].clone();
        f.edit(&mut epoch, 2);
        let saved = epoch.snapshot().unwrap();
        let mut epoch = f.restore(&saved).unwrap();
        let retry = epoch
            .edit_or_reseal(&f.owner, &f.group, &mut f.rng, &first)
            .unwrap();
        let key = f
            .group
            .channel_secret(&f.owner, DocType::DocRegistry, epoch.doc_id())
            .unwrap();
        assert_eq!(retry.open(&key).unwrap(), original);
        assert_eq!(epoch.snapshot().unwrap(), saved);
        assert_eq!(epoch.op_count(), 2);
        // A marker is not evidence of equality: the id deliberately does not hash the body.
        let mut conflicting = f.domain(2);
        conflicting.nonce = first.nonce;
        assert!(matches!(
            epoch.validate_local_edit(&f.owner, &f.group, &conflicting),
            Err(ReplError::IntentConflict)
        ));
        assert!(matches!(
            epoch.edit_or_reseal(&f.owner, &f.group, &mut f.rng, &conflicting),
            Err(ReplError::IntentConflict)
        ));
        assert_eq!(epoch.snapshot().unwrap(), saved);
    }

    #[test]
    fn registry_local_retry_requires_current_author_actor_scope_and_open_gate() {
        let mut f = Fixture::new();
        let mut epoch = f.empty();
        let domain = f.domain(1);
        f.edit(&mut epoch, 1);
        let next = MlsDevice::generate().unwrap();
        let welcome = f
            .group
            .add_member(&f.owner, next.key_package().unwrap())
            .unwrap()
            .welcome;
        let mut group = ServerGroup::join(&next, &welcome).unwrap();
        assert!(
            matches!(
                epoch.edit_or_reseal(&next, &group, &mut f.rng, &domain),
                Err(ReplError::EpochAuthority)
            ),
            "another admitted member cannot use this actor"
        );
        let wrong_group = ServerGroup::create(&f.owner).unwrap();
        assert!(matches!(
            epoch.validate_local_edit(&f.owner, &wrong_group, &domain),
            Err(ReplError::EpochScope)
        ));
        epoch.seal(f.receipt(&epoch, 7), &f.group, 0).unwrap();
        assert!(matches!(
            epoch.edit_or_reseal(&f.owner, &f.group, &mut f.rng, &domain),
            Err(ReplError::EpochClosed)
        ));
        group.remove_member(&next, &f.owner.device_id()).unwrap();
        assert!(
            matches!(
                epoch.validate_local_edit(&f.owner, &group, &domain),
                Err(ReplError::EpochAuthority)
            ),
            "a removed original author cannot retry"
        );
    }

    #[test]
    fn registry_epoch_empty_and_edited_restart_preserve_exact_bytes_and_writer() {
        let mut f = Fixture::new();
        let mut epoch = f.empty();
        for n in 0..3 {
            let saved = epoch.snapshot().unwrap();
            epoch = f.restore(&saved).unwrap();
            assert_eq!(epoch.snapshot().unwrap(), saved);
            assert_eq!(epoch.phase(), EpochPhase::Open);
            assert_eq!(epoch.op_count(), usize::from(n));
            f.edit(&mut epoch, n);
        }
        assert_eq!(epoch.projection().unwrap().pointers[&f.key], 2);
    }

    #[test]
    fn registry_epoch_closing_retains_full_log_and_duplicate_late_hash_survives_restart() {
        let mut f = Fixture::new();
        let mut source = f.empty();
        let mut receiver = f.empty();
        let accepted = f.edit(&mut source, 1);
        assert_eq!(
            receiver.ingest(&accepted, &f.group, &f.owner).unwrap(),
            Admission::Accepted
        );
        let receipt = f.receipt(&receiver, 7);
        assert_eq!(
            receiver.seal(receipt.clone(), &f.group, 0).unwrap(),
            ReceiptIngest::Advanced
        );
        assert_eq!(
            receiver.seal(receipt, &f.group, 0).unwrap(),
            ReceiptIngest::Duplicate
        );
        let late = f.edit(&mut source, 2);
        for _ in 0..3 {
            assert_eq!(
                receiver.ingest(&late, &f.group, &f.owner).unwrap(),
                Admission::Quarantined
            );
        }
        let saved = receiver.snapshot().unwrap();
        let mut receiver = f.restore(&saved).unwrap();
        assert_eq!(receiver.snapshot().unwrap(), saved);
        assert_eq!(receiver.phase(), EpochPhase::Closing);
        assert_eq!(receiver.op_count(), 1);
        assert_eq!(receiver.quarantined_len(), 1);
        assert_eq!(receiver.projection().unwrap().pointers[&f.key], 1);
        assert_eq!(
            receiver.ingest(&accepted, &f.group, &f.owner).unwrap(),
            Admission::Duplicate
        );
        let domain = f.domain(3);
        assert!(matches!(
            receiver.edit(&f.owner, &f.group, &mut f.rng, &domain),
            Err(ReplError::EpochClosed)
        ));
    }

    #[test]
    fn registry_epoch_conflicting_receipts_restore_a_read_only_fault_with_source_intact() {
        let mut f = Fixture::new();
        let mut epoch = f.empty();
        f.edit(&mut epoch, 1);
        epoch.seal(f.receipt(&epoch, 7), &f.group, 0).unwrap();
        assert_eq!(
            epoch.seal(f.receipt(&epoch, 8), &f.group, 0).unwrap(),
            ReceiptIngest::Fault
        );
        let mut restored = f.restore(&epoch.snapshot().unwrap()).unwrap();
        assert_eq!(restored.phase(), EpochPhase::Fault);
        assert_eq!(restored.op_count(), 1);
        let domain = f.domain(2);
        assert!(matches!(
            restored.edit(&f.owner, &f.group, &mut f.rng, &domain),
            Err(ReplError::EpochClosed)
        ));
    }

    #[test]
    fn registry_epoch_checkpoint_restart_binds_seed_opening_receipt_and_separate_dag() {
        let mut f = Fixture::new();
        let mut source = f.empty();
        f.edit(&mut source, 1);
        let seed = source.projection().unwrap().checkpoint([7; 32]).unwrap();
        let receipt = f.receipt(&source, 7);
        let mut successor = RegistryEpoch::from_checkpoint(
            &f.group,
            f.key.bucket(),
            f.owner.device_id(),
            receipt,
            0,
            seed.bytes(),
        )
        .unwrap();
        let source_change = automerge::Change::from_bytes(source.doc.signed_log()[0].delta.clone())
            .unwrap()
            .hash();
        f.edit(&mut successor, 2);
        let mut restored = f.restore(&successor.snapshot().unwrap()).unwrap();
        assert_eq!(restored.epoch(), 1);
        assert_eq!(restored.op_count(), 1);
        let change =
            automerge::Change::from_bytes(restored.doc.signed_log()[0].delta.clone()).unwrap();
        assert_eq!(change.deps(), &[automerge::ChangeHash(seed.change_hash())]);
        assert!(restored
            .doc
            .doc()
            .clone()
            .get_change_by_hash(&source_change)
            .is_none());
        restored.seal(f.receipt(&restored, 8), &f.group, 0).unwrap();
        assert_eq!(
            f.restore(&restored.snapshot().unwrap()).unwrap().phase(),
            EpochPhase::Closing
        );
        assert_eq!(
            source.phase(),
            EpochPhase::Open,
            "construction never retires predecessor"
        );
        assert_eq!(source.op_count(), 1);
    }

    #[test]
    fn registry_epoch_rejects_mismatched_gate_log_metadata_and_receipt_phase() {
        let mut f = Fixture::new();
        let mut epoch = f.empty();
        f.edit(&mut epoch, 1);
        let saved = epoch.snapshot().unwrap();
        // All splices consist of individually valid encoded pieces, as a buggy multi-file save
        // might produce. Authenticating each file alone would not detect these inconsistencies.
        epoch.gate = f.empty().gate;
        assert!(f.restore(&epoch.snapshot().unwrap()).is_err());
        epoch = f.restore(&saved).unwrap();
        epoch.doc = f.empty().doc;
        assert!(f.restore(&epoch.snapshot().unwrap()).is_err());
        epoch = f.restore(&saved).unwrap();
        let op = &epoch.doc.signed_log()[0];
        let metadata = AdmittedOperation {
            op_hash: op.hash(),
            domain_op_id: [9; 32],
            author: op.author_device,
            encoded_len: op.encode().len(),
        };
        epoch.gate = f.empty().gate;
        epoch.gate.admit_local(metadata).unwrap();
        assert!(f.restore(&epoch.snapshot().unwrap()).is_err());
        epoch = f.restore(&saved).unwrap();
        epoch.seal(f.receipt(&epoch, 7), &f.group, 0).unwrap();
        epoch.receipts = ReceiptBook::default();
        assert!(f.restore(&epoch.snapshot().unwrap()).is_err());
    }

    #[test]
    fn registry_epoch_scope_and_fresh_receipt_authority_cannot_be_spliced() {
        let f = Fixture::new();
        let mut epoch = f.empty();
        let saved = epoch.snapshot().unwrap();
        let outsider = MlsDevice::generate().unwrap();
        let other_group = ServerGroup::create(&outsider).unwrap();
        assert!(
            RegistryEpoch::restore(&saved, &other_group, f.key.bucket(), outsider.device_id())
                .is_err()
        );
        assert!(RegistryEpoch::restore(
            &saved,
            &f.group,
            f.key.bucket().wrapping_add(1),
            f.owner.device_id()
        )
        .is_err());
        let forged = Receipt::sign(
            epoch.logical.clone(),
            0,
            [1; 32],
            [2; 32],
            0,
            InheritedCheckpoint::EpochZero,
            &outsider,
        )
        .unwrap();
        assert!(matches!(
            epoch.seal(forged, &f.group, 0),
            Err(ReplError::EpochAuthority)
        ));
        assert!(matches!(
            epoch.seal(f.receipt(&epoch, 7), &f.group, 1),
            Err(ReplError::EpochAuthority)
        ));
        assert_eq!(epoch.snapshot().unwrap(), saved);
    }

    #[test]
    fn registry_epoch_restore_rejects_missing_dependency_duplicate_and_corrupt_signature() {
        let mut f = Fixture::new();
        let mut epoch = f.empty();
        f.edit(&mut epoch, 1);
        f.edit(&mut epoch, 2);
        let saved = epoch.snapshot().unwrap();
        let rewrite = |mode: u8| {
            let mut d = Decoder::new(&saved);
            let mut e = Encoder::new();
            e.put_u8(d.get_u8().unwrap());
            e.put_u8(d.get_u8().unwrap());
            for _ in 0..4 {
                e.put_bytes(d.get_bytes().unwrap()).unwrap();
            }
            assert_eq!(d.get_u32().unwrap(), 2);
            let first = d.get_bytes().unwrap();
            let second = d.get_bytes().unwrap();
            match mode {
                0 => {
                    e.put_u32(1);
                    e.put_bytes(second).unwrap();
                }
                1 => {
                    e.put_u32(2);
                    e.put_bytes(first).unwrap();
                    e.put_bytes(first).unwrap();
                }
                _ => {
                    let mut op = SignedOp::decode(first).unwrap();
                    op.signature[0] ^= 1;
                    e.put_u32(2);
                    e.put_bytes(&op.encode()).unwrap();
                    e.put_bytes(second).unwrap();
                }
            }
            e.finish()
        };
        for mode in 0..3 {
            assert!(f.restore(&rewrite(mode)).is_err());
        }
    }

    #[test]
    fn registry_epoch_restore_checks_framing_and_caps_before_parsing_bodies() {
        let f = Fixture::new();
        let mut epoch = f.empty();
        let saved = epoch.snapshot().unwrap();
        for len in 0..saved.len() {
            assert!(f.restore(&saved[..len]).is_err());
        }
        let mut trailing = saved.clone();
        trailing.push(0);
        assert!(f.restore(&trailing).is_err());
        let mut bad_version = saved;
        bad_version[0] = 2;
        assert!(f.restore(&bad_version).is_err());
        assert!(matches!(
            f.restore(&vec![0; MAX_REGISTRY_EPOCH_SNAPSHOT_BYTES + 1]),
            Err(ReplError::EpochBound)
        ));
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_u8(f.key.bucket());
        e.put_bytes(&vec![0; MAX_RECEIPT_BYTES + 1]).unwrap();
        assert!(matches!(f.restore(&e.finish()), Err(ReplError::EpochBound)));
    }

    #[test]
    fn registry_epoch_historical_checkpoint_and_removed_author_survive_succession() {
        let mut f = Fixture::new();
        let mut source = f.empty();
        f.edit(&mut source, 1);
        let seed = source.projection().unwrap().checkpoint([7; 32]).unwrap();
        let receipt = f.receipt(&source, 7);
        let mut checkpoint = RegistryEpoch::from_checkpoint(
            &f.group,
            f.key.bucket(),
            f.owner.device_id(),
            receipt.clone(),
            0,
            seed.bytes(),
        )
        .unwrap();
        f.edit(&mut checkpoint, 2);
        let saved = checkpoint.snapshot().unwrap();
        // Real MLS succession: historical owner is no longer a member. A local restart keeps
        // their already verified content, but fresh network installation still rejects it.
        let next = MlsDevice::generate().unwrap();
        let welcome = f
            .group
            .add_member(&f.owner, next.key_package().unwrap())
            .unwrap()
            .welcome;
        let mut next_group = ServerGroup::join(&next, &welcome).unwrap();
        next_group
            .remove_member(&next, &f.owner.device_id())
            .unwrap();
        let mut restored =
            RegistryEpoch::restore(&saved, &next_group, f.key.bucket(), next.device_id()).unwrap();
        assert_eq!(restored.projection().unwrap().pointers[&f.key], 2);
        assert_eq!(restored.op_count(), 1);
        assert!(RegistryEpoch::from_checkpoint(
            &next_group,
            f.key.bucket(),
            next.device_id(),
            receipt,
            0,
            seed.bytes()
        )
        .is_err());
        let domain = f.domain(3);
        restored
            .edit(&next, &next_group, &mut f.rng, &domain)
            .unwrap();
        assert_eq!(restored.op_count(), 2);
        let saved = restored.snapshot().unwrap();
        assert_eq!(
            RegistryEpoch::restore(&saved, &next_group, f.key.bucket(), next.device_id())
                .unwrap()
                .op_count(),
            2
        );
    }

    #[test]
    fn registry_epoch_seed_or_opening_receipt_splice_never_creates_a_root() {
        let mut f = Fixture::new();
        let mut source = f.empty();
        f.edit(&mut source, 1);
        let seed = source.projection().unwrap().checkpoint([7; 32]).unwrap();
        let mut checkpoint = RegistryEpoch::from_checkpoint(
            &f.group,
            f.key.bucket(),
            f.owner.device_id(),
            f.receipt(&source, 7),
            0,
            seed.bytes(),
        )
        .unwrap();
        let saved = checkpoint.snapshot().unwrap();
        // Substitute another valid receipt from the very same owner/scope, then an empty seed.
        checkpoint.opening = Some(f.receipt(&source, 8));
        assert!(f.restore(&checkpoint.snapshot().unwrap()).is_err());
        let mut d = Decoder::new(&saved);
        let mut e = Encoder::new();
        e.put_u8(d.get_u8().unwrap());
        e.put_u8(d.get_u8().unwrap());
        e.put_bytes(d.get_bytes().unwrap()).unwrap();
        d.get_bytes().unwrap();
        e.put_bytes(&[]).unwrap();
        e.put_bytes(d.get_bytes().unwrap()).unwrap();
        e.put_bytes(d.get_bytes().unwrap()).unwrap();
        e.put_u32(0);
        assert!(f.restore(&e.finish()).is_err());
    }

    #[test]
    fn registry_epoch_cross_epoch_inheritance_fault_survives_restart() {
        let f = Fixture::new();
        let source = f.empty();
        let seed = source.projection().unwrap().checkpoint([7; 32]).unwrap();
        let mut epoch = RegistryEpoch::from_checkpoint(
            &f.group,
            f.key.bucket(),
            f.owner.device_id(),
            f.receipt(&source, 7),
            0,
            seed.bytes(),
        )
        .unwrap();
        let changed = Receipt::sign(
            epoch.logical.clone(),
            1,
            [8; 32],
            [9; 32],
            0,
            InheritedCheckpoint::Checkpoint {
                epoch: 1,
                close_record_hash: [7; 32],
                seed_change_hash: seed.change_hash(),
            },
            &f.owner,
        )
        .unwrap();
        assert_eq!(
            epoch.seal(changed, &f.group, 0).unwrap(),
            ReceiptIngest::Fault
        );
        let saved = epoch.snapshot().unwrap();
        let mut restored = f.restore(&saved).unwrap();
        assert_eq!(restored.phase(), EpochPhase::Fault);
        assert_eq!(restored.snapshot().unwrap(), saved);
    }

    #[test]
    fn registry_epoch_new_owner_cannot_replace_a_pending_seal_without_recovery() {
        let mut f = Fixture::new();
        let mut epoch = f.empty();
        epoch.seal(f.receipt(&epoch, 7), &f.group, 0).unwrap();
        let next = MlsDevice::generate().unwrap();
        let welcome = f
            .group
            .add_member(&f.owner, next.key_package().unwrap())
            .unwrap()
            .welcome;
        let mut next_group = ServerGroup::join(&next, &welcome).unwrap();
        next_group
            .remove_member(&next, &f.owner.device_id())
            .unwrap();
        let signed = Receipt::sign(
            epoch.logical.clone(),
            0,
            [8; 32],
            [9; 32],
            next_group.epoch(),
            InheritedCheckpoint::EpochZero,
            &next,
        )
        .unwrap();
        // Account for the legitimate quota-owner refresh, then prove no receipt/state is lost.
        epoch.refresh_owner(&next_group).unwrap();
        let before = epoch.snapshot().unwrap();
        assert!(matches!(
            epoch.seal(signed, &next_group, next_group.epoch()),
            Err(ReplError::ReceiptConflict)
        ));
        assert_eq!(epoch.snapshot().unwrap(), before);
        assert_eq!(
            RegistryEpoch::restore(&before, &next_group, f.key.bucket(), next.device_id())
                .unwrap()
                .phase(),
            EpochPhase::Closing
        );
    }
}
