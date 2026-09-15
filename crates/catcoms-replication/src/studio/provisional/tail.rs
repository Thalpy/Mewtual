//! Current-member tail validation over a private unconfirmed graph. This never constructs an
//! EncryptedDoc, epoch gate, checkpoint origin or historical-authority capability.
use super::*;
use crate::epoch::{MAX_EPOCH_BYTES, MAX_EPOCH_OPERATIONS, MAX_SIGNED_EPOCH_OP_BYTES};
use crate::studio::epoch::catchup::{MAX_STUDIO_PAGE_BYTES, MAX_STUDIO_PAGE_OPS};
use crate::{LocalIntent, SealedOp, SignedOp};
use automerge::{Change, ReadDoc, ScalarValue, Value, ROOT};
use catcoms_mls::{MlsDevice, ServerGroup};

pub struct UnconfirmedStudioTailPreparation {
    seed: UnconfirmedStudioSeed,
    operations: Vec<SignedOp>,
}
impl std::fmt::Debug for UnconfirmedStudioTailPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UnconfirmedStudioTailPreparation { .. }")
    }
}
impl UnconfirmedStudioSeed {
    /// Authenticate at most one bounded page under current membership. No MLS borrow or keys
    /// survive into cold Automerge/typed parsing. The caller must recheck that same membership
    /// epoch and the original candidate lifetime before using the completed preparation.
    pub fn prepare_tail(
        self,
        sealed: Vec<SealedOp>,
        group: &ServerGroup,
        device: &MlsDevice,
    ) -> Result<UnconfirmedStudioTailPreparation, ReplError> {
        if self.projection.document().server_id != group.group_id() {
            return Err(ReplError::EpochScope);
        }
        if sealed.len() > MAX_STUDIO_PAGE_OPS {
            return Err(ReplError::EpochBound);
        }
        let mut wire_bytes = 0usize;
        for op in &sealed {
            // Refuse oversized ciphertext before encode() can allocate a second copy. Include
            // each length prefix in the same aggregate rail as the shared wire codec.
            if op.blob.ciphertext.len() > MAX_SIGNED_EPOCH_OP_BYTES + 20 {
                return Err(ReplError::EpochBound);
            }
            wire_bytes = wire_bytes.saturating_add(4 + op.encode().len());
            if wire_bytes > MAX_STUDIO_PAGE_BYTES {
                return Err(ReplError::EpochBound);
            }
        }
        let doc_type = self.projection.document().doc_type;
        let key = zeroize::Zeroizing::new(group.channel_secret(device, doc_type, self.doc_id)?);
        let mut operations = Vec::with_capacity(sealed.len());
        for sealed in sealed {
            if sealed.doc_type != doc_type || sealed.doc_id != self.doc_id {
                return Err(ReplError::EpochScope);
            }
            if sealed.epoch != group.epoch() {
                return Err(ReplError::EpochUnavailable(sealed.epoch));
            }
            let op = sealed.open(&key)?;
            // A relay holding the sealing key cannot attest a removed author's history.
            if group.member_signature_key(&op.author_device).as_deref()
                != Some(op.author_pubkey.as_slice())
            {
                return Err(ReplError::EpochAuthority);
            }
            if op.doc_type != doc_type || op.doc_id != self.doc_id {
                return Err(ReplError::EpochScope);
            }
            if op.encode().len() > MAX_SIGNED_EPOCH_OP_BYTES {
                return Err(ReplError::EpochBound);
            }
            if !op.verify() {
                return Err(ReplError::BadSignature);
            }
            operations.push(op);
        }
        Ok(UnconfirmedStudioTailPreparation {
            seed: self,
            operations,
        })
    }
    fn apply_tail_operation(&mut self, op: SignedOp) -> Result<(), ReplError> {
        let hash = op.hash();
        if self.applied.contains(&hash) {
            return Ok(());
        }
        let encoded_len = op.encode().len();
        if self.applied.len() == MAX_EPOCH_OPERATIONS
            || self.encoded_bytes.saturating_add(encoded_len) > MAX_EPOCH_BYTES
        {
            return Err(ReplError::EpochBound);
        }
        let domain = op.parsed_domain_op()?.ok_or(ReplError::Malformed)?;
        let logical = self.projection.document();
        if domain.doc_type != logical.doc_type || domain.logical_key != logical.logical_key {
            return Err(ReplError::EpochScope);
        }
        let operation_id = domain.id(&op.author_device);
        if self.operations.contains_key(&operation_id) {
            return Err(ReplError::IntentConflict);
        }
        let change = Change::from_bytes(op.delta.clone())
            .map_err(|e| ReplError::Automerge(e.to_string()))?;
        if change.actor_id().to_bytes() != op.author_device.as_bytes() {
            return Err(ReplError::EpochAuthority);
        }
        // Every accepted change must descend from the exact seed, inductively through already
        // checked predecessors. Automerge's missing-dependency buffering is not admission.
        if change.deps().is_empty()
            || change
                .deps()
                .iter()
                .any(|h| self.doc.get_change_by_hash(h).is_none())
        {
            return Err(ReplError::EpochScope);
        }
        self.target.validate(
            logical,
            self.projection.epoch(),
            &domain,
            &change,
            &self.doc,
        )?;
        let mut staged = self.doc.clone();
        staged
            .load_incremental(&op.delta)
            .map_err(|e| ReplError::Automerge(e.to_string()))?;
        let marker = crate::doc::domain_marker_key(&operation_id);
        if !staged.get(ROOT, marker).map_err(|e| ReplError::Automerge(e.to_string()))?
            .is_some_and(|(value, _)| matches!(value, Value::Scalar(v) if v.as_ref() == &ScalarValue::Uint(1)))
        {
            return Err(ReplError::Malformed);
        }
        let projection = self
            .target
            .read(logical, self.projection.epoch(), &staged)?;
        self.operations.insert(
            operation_id,
            LocalIntent {
                author: op.author_device,
                operation: domain,
            },
        );
        super::super::recovery::preflight(projection.clone(), &self.operations)?;
        self.projection = projection;
        self.doc = staged;
        self.applied.insert(hash);
        self.encoded_bytes += encoded_len;
        Ok(())
    }
}
impl UnconfirmedStudioTailPreparation {
    /// Consumes the entire candidate on any failure, so no partially checked page is usable.
    pub fn prepare(mut self) -> Result<UnconfirmedStudioSeed, ReplError> {
        for op in self.operations {
            self.seed.apply_tail_operation(op)?;
        }
        Ok(self.seed)
    }
}
