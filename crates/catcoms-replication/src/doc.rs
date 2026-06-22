//! [`EncryptedDoc`] — one encrypted, replicated CRDT document.
//!
//! A document (a channel, wiki page, status feed, calendar) is an automerge
//! document plus an append-only log of the [`SignedOp`]s that built it. Local
//! edits produce a [`SealedOp`] to broadcast; inbound sealed ops are decrypted,
//! their inner signature verified, then applied (automerge buffers any that
//! arrive before their dependencies). For a member who joined late, a current
//! member exports the signed-op log **re-sealed under the current epoch**, so the
//! latecomer converges without ever needing old epoch keys (forward secrecy is
//! preserved) while still verifying each op's original authorship.

use std::collections::HashSet;
use std::fmt;

use automerge::{ActorId, AutoCommit};
use catcoms_crypto::DeviceId;
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, DocType, Encoder};

use crate::op::{SealedOp, SignedOp};
use crate::ReplError;

/// One encrypted, replicated CRDT document.
pub struct EncryptedDoc {
    doc_type: DocType,
    doc_id: u128,
    doc: AutoCommit,
    log: Vec<SignedOp>,
    applied: HashSet<[u8; 32]>,
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
        }
    }

    /// Borrow the underlying automerge document (for reads/projection).
    pub fn doc(&self) -> &AutoCommit {
        &self.doc
    }

    /// Number of ops in this document's log.
    pub fn op_count(&self) -> usize {
        self.log.len()
    }

    /// Serialize this document for persistence (Phase 9d): the materialized automerge state
    /// plus the signed-op log (the log carries the per-op signatures the automerge state
    /// does not, so a restored member can still serve catch-up). The `applied` dedup set is
    /// rebuilt from the log on restore. **Secret** — holds plaintext document content; the
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
        })
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
        edit(&mut self.doc).map_err(|e| ReplError::Automerge(e.to_string()))?;
        self.doc.commit();
        let change = self
            .doc
            .get_last_local_change()
            .ok_or(ReplError::NoChange)?;
        let delta = change.raw_bytes().to_vec();

        let op = SignedOp::sign(device, self.doc_type, self.doc_id, delta)?;
        let sealed = SealedOp::seal(&op, group, device, rng)?;
        self.record(op);
        Ok(sealed)
    }

    /// Decrypt, verify and apply an inbound sealed op. Returns `true` if it was
    /// newly applied, `false` if it was a duplicate.
    pub fn ingest(
        &mut self,
        sealed: &SealedOp,
        group: &ServerGroup,
        device: &MlsDevice,
    ) -> Result<bool, ReplError> {
        self.check_doc(sealed.doc_type, sealed.doc_id)?;
        if sealed.epoch != group.epoch() {
            return Err(ReplError::EpochUnavailable(sealed.epoch));
        }
        let key = group.channel_secret(device, self.doc_type, self.doc_id)?;
        let op = sealed.open(&key)?;
        self.apply_signed(op)
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
    /// applied — so this cannot be used to inject forged history. The caller pairs
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
        self.check_doc(sealed.doc_type, sealed.doc_id)?;
        if sealed.epoch != expected_epoch {
            return Err(ReplError::EpochUnavailable(sealed.epoch));
        }
        let op = sealed.open(key)?;
        self.apply_signed(op)
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
        let key = group.channel_secret(device, self.doc_type, self.doc_id)?;
        let mut applied = 0;
        for sealed in ops {
            self.check_doc(sealed.doc_type, sealed.doc_id)?;
            if sealed.epoch != group.epoch() {
                return Err(ReplError::EpochUnavailable(sealed.epoch));
            }
            let op = sealed.open(&key)?;
            if self.apply_signed(op)? {
                applied += 1;
            }
        }
        Ok(applied)
    }

    fn apply_signed(&mut self, op: SignedOp) -> Result<bool, ReplError> {
        self.check_doc(op.doc_type, op.doc_id)?;
        let hash = op.hash();
        if self.applied.contains(&hash) {
            return Ok(false);
        }
        if !op.verify() {
            return Err(ReplError::BadSignature);
        }
        self.doc
            .load_incremental(&op.delta)
            .map_err(|e| ReplError::Automerge(e.to_string()))?;
        self.applied.insert(hash);
        self.log.push(op);
        Ok(true)
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
}
