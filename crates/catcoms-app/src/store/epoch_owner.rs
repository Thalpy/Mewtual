//! Vault-backed owner decisions. This is the persist-before-publish barrier, not the network
//! publisher or settlement coordinator. Reads expose historical state only; every publication
//! must first re-save its decision and recheck current owner/tenure at the actual send boundary.

use std::io::Read;

use catcoms_mls::ServerGroup;
use catcoms_replication::{
    epoch::{MAX_CLOSE_RECORD_BYTES, MAX_OWNER_RECEIPT_JOURNAL_BYTES, MAX_RECEIPT_BYTES},
    registry_epoch::RegistryOwnerDecision,
    CloseRecord, LogicalDocument, OwnerReceiptJournal, Receipt,
};

use super::epoch_budget::{
    EpochStorageBudget, Footprint, Replacement, StorageRecord, StorageScope, WritePurpose,
};
use super::epoch_recovery::inventory::{is_link, regular_file};
use super::epoch_recovery::AuthenticatedEpochFileBytes;
use super::*;

pub(super) const RECORD_DOMAIN: &[u8] = b"catcoms/epoch-owner-store/v1";
// Full framed local/group/type/key scope plus length framing, separate from the signed wire.
const MAX_RECORD_BYTES: usize = MAX_OWNER_RECEIPT_JOURNAL_BYTES + MAX_CLOSE_RECORD_BYTES + 1024;
pub(super) const MAX_SEALED_BYTES: usize = MAX_RECORD_BYTES + 40;

/// Historical, authenticated journal view. It is not a publication permit or pruning authority.
/// The state has no mutation API: transitions must reload the current on-disk journal.
#[derive(Default)]
pub struct EpochOwnerReceiptState {
    journal: OwnerReceiptJournal,
    // One exact close for the pending-preferred decision. Retained across publication completion
    // so a crash before source installation never needs to reconstruct lost close heads.
    decision_close: Option<([u8; 32], CloseRecord)>,
}

impl std::fmt::Debug for EpochOwnerReceiptState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochOwnerReceiptState")
            .field("pending", &self.pending().is_some())
            .field("published", &self.published().is_some())
            .finish_non_exhaustive()
    }
}

impl EpochOwnerReceiptState {
    /// Exact pending signed decision. After restart, re-prepare this before republication;
    /// merely reading visible bytes does not repair an earlier directory-sync failure.
    pub fn pending(&self) -> Option<&Receipt> {
        self.journal.in_flight()
    }

    /// Last completed publication. It may belong to an earlier owner tenure.
    pub fn published(&self) -> Option<&Receipt> {
        self.journal.published()
    }

    /// Historical close for this exact saved receipt, not fresh publication authority. Legacy
    /// receipt-only journals return None: a pending choice must hold, never be regenerated.
    pub fn close_for(&self, receipt: &Receipt) -> Option<&CloseRecord> {
        self.decision_close
            .as_ref()
            .filter(|(hash, _)| *hash == receipt.hash())
            .map(|(_, close)| close)
    }

    fn check_scope(&self, document: &LogicalDocument) -> Result<(), AppError> {
        if self
            .pending()
            .into_iter()
            .chain(self.published())
            .any(|r| &r.document != document)
        {
            return Err(invalid("journal belongs to another logical document"));
        }
        if let Some((hash, close)) = &self.decision_close {
            let receipt = self
                .pending()
                .or_else(|| self.published())
                .ok_or_else(|| invalid("close without a decision"))?;
            if close.server_id.len() > 256
                || close.author_public_key.len() != 32
                || close.heads.len() > 64
            {
                return Err(invalid("saved close exceeds its bound"));
            }
            CloseRecord::decode(&close.encode()).map_err(invalid)?;
            if *hash != receipt.hash()
                || close.hash() != receipt.close_record_hash
                || close.closed_epoch != receipt.closed_epoch
                || close.server_id != document.server_id
                || close.doc_type != document.doc_type
            {
                return Err(invalid("saved close does not bind the selected receipt"));
            }
        }
        Ok(())
    }

    fn encode(
        &self,
        scope: &[u8],
        document: &LogicalDocument,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.check_scope(document)?;
        let journal = Zeroizing::new(self.journal.encode());
        if journal.len() > MAX_OWNER_RECEIPT_JOURNAL_BYTES {
            return Err(invalid("journal exceeds its bound"));
        }
        // Never acknowledge a write which the bounded restart decoder would refuse.
        OwnerReceiptJournal::decode(&journal).map_err(invalid)?;
        let mut e = Encoder::new();
        e.put_bytes(scope).map_err(invalid)?;
        e.put_bytes(&journal).map_err(invalid)?;
        if let Some((hash, close)) = &self.decision_close {
            // Existing receipt-only records remain byte-identical. Old readers reject this
            // explicit extension rather than silently losing irrevocable decision provenance.
            e.put_u8(2);
            e.put_bytes(hash).map_err(invalid)?;
            e.put_bytes(&close.encode()).map_err(invalid)?;
        }
        Ok(Zeroizing::new(e.finish()))
    }

    pub(super) fn decode(
        bytes: &[u8],
        scope: &[u8],
        document: &LogicalDocument,
    ) -> Result<Self, AppError> {
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(invalid("record exceeds its bound"));
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes().map_err(invalid)? != scope {
            return Err(invalid("record belongs to another server or document"));
        }
        let journal =
            OwnerReceiptJournal::decode(d.get_bytes().map_err(invalid)?).map_err(invalid)?;
        let decision_close = if d.is_empty() {
            None
        } else {
            if d.get_u8().map_err(invalid)? != 2 {
                return Err(invalid("unsupported owner decision extension"));
            }
            let hash = d
                .get_bytes()
                .map_err(invalid)?
                .try_into()
                .map_err(invalid)?;
            let close = CloseRecord::decode(d.get_bytes().map_err(invalid)?).map_err(invalid)?;
            Some((hash, close))
        };
        d.finish().map_err(invalid)?;
        let state = Self {
            journal,
            decision_close,
        };
        state.check_scope(document)?;
        Ok(state)
    }
}

impl ServerStore {
    /// Load historical owner state, bounded before decrypting. Only absence is empty; corruption
    /// never resets an irrevocable choice. This does not re-authorize old signatures for sending.
    pub fn load_epoch_owner_receipts(
        &self,
        server: u64,
        document: &LogicalDocument,
    ) -> Result<EpochOwnerReceiptState, AppError> {
        let scope = scope_bytes(server, document)?;
        self.read_epoch_owner_record(&scope, document)
            .map(|(state, _)| state)
    }

    /// One authenticated final-file accounting input, NOT a full inventory. The future exclusive
    /// coordinator must also discover all documents, other managed types and orphan siblings.
    /// Recovery-only scans deliberately do not cover this separate `.owner-receipts` namespace.
    pub fn epoch_owner_receipt_inventory_record(
        &self,
        server: u64,
        document: &LogicalDocument,
    ) -> Result<Option<StorageRecord>, AppError> {
        let scope = scope_bytes(server, document)?;
        let (_, size) = self.read_epoch_owner_record(&scope, document)?;
        size.map(|size| storage_record(server, document, &scope, size))
            .transpose()
    }

    /// Persist an owner-authenticated decision before returning it for publication. The caller
    /// supplies tenure evidence from locally observed succession, never from the receipt itself.
    /// An exact retry re-saves an already-published choice only while no different decision is
    /// pending; otherwise the publisher must resume that pending choice. A failed/uncertain write
    /// returns no success and blocks the budget until complete inventory reconciliation.
    ///
    /// This checks signature/owner/tenure, not the close's closure or checkpoint materialization;
    /// the registry owner driver validates those before signing; other callers must do so too.
    /// Sending must recheck current authority/session after this call. Success alone authorizes
    /// no history pruning.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_epoch_owner_receipt(
        &mut self,
        server: u64,
        receipt: Receipt,
        group: &ServerGroup,
        expected_tenure_start_group_epoch: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<EpochOwnerReceiptState, AppError> {
        self.prepare_epoch_owner_with_writer(
            server,
            receipt,
            group,
            expected_tenure_start_group_epoch,
            rng,
            budget,
            atomic_write,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_epoch_owner_with_writer(
        &mut self,
        server: u64,
        receipt: Receipt,
        group: &ServerGroup,
        tenure: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
    ) -> Result<EpochOwnerReceiptState, AppError> {
        // Public Receipt fields are not a validation boundary. Bound variable fields before
        // canonical encoding/signature work, then use the wire decoder's exact schema too.
        scope_bytes(server, &receipt.document)?;
        if receipt.owner_public_key.len() != 32 || receipt.encode().len() > MAX_RECEIPT_BYTES {
            return Err(invalid("receipt exceeds its bound"));
        }
        let receipt = Receipt::decode(&receipt.encode()).map_err(invalid)?;
        let document = receipt.document.clone();
        self.update_epoch_owner_with_writer(
            server,
            &document,
            rng,
            budget,
            |journal| journal.prepare(receipt, group, tenure).map_err(invalid),
            writer,
        )
    }

    /// Record completion of the exact saved publication. Wrong/stale hashes cannot clear a newer
    /// pending decision. An exact completed retry still writes before returning, repairing the
    /// visible-but-not-durable case. No receipt is sent by this method.
    pub fn mark_epoch_owner_receipt_published(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        receipt_hash: [u8; 32],
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<EpochOwnerReceiptState, AppError> {
        self.update_epoch_owner_with_writer(
            server,
            document,
            rng,
            budget,
            |journal| journal.mark_published(receipt_hash).map_err(invalid),
            atomic_write,
        )
    }

    // Reload + transition + reservation + durable replacement share the exclusive store borrow.
    // There is deliberately no public unaccounted owner-journal save or caller-provided writer.
    fn update_epoch_owner_with_writer(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        apply: impl FnOnce(&mut OwnerReceiptJournal) -> Result<(), AppError>,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
    ) -> Result<EpochOwnerReceiptState, AppError> {
        self.update_epoch_owner_state_with_writer(
            server,
            document,
            rng,
            budget,
            |state| {
                apply(&mut state.journal)?;
                // Legacy callers may prepare a new decision without its close. Keep an existing
                // close only for the still-selected receipt; never attach old heads to a new choice.
                if state.decision_close.as_ref().is_some_and(|(hash, _)| {
                    state
                        .pending()
                        .or_else(|| state.published())
                        .is_none_or(|r| r.hash() != *hash)
                }) {
                    state.decision_close = None;
                }
                Ok(())
            },
            writer,
        )
    }

    /// Save both halves of a validated registry decision in one accounted atomic record. This
    /// remains crate-private: production callers derive it from the checked, gated source.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_registry_owner_decision(
        &mut self,
        server: u64,
        decision: &RegistryOwnerDecision,
        group: &ServerGroup,
        tenure: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<EpochOwnerReceiptState, AppError> {
        self.prepare_registry_owner_decision_with_writer(
            server,
            decision,
            group,
            tenure,
            rng,
            budget,
            atomic_write,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_registry_owner_decision_with_writer(
        &mut self,
        server: u64,
        decision: &RegistryOwnerDecision,
        group: &ServerGroup,
        tenure: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
    ) -> Result<EpochOwnerReceiptState, AppError> {
        self.update_epoch_owner_state_with_writer(
            server,
            &decision.receipt().document,
            rng,
            budget,
            |state| {
                state
                    .journal
                    .prepare(decision.receipt().clone(), group, tenure)
                    .map_err(invalid)?;
                state.decision_close = Some((decision.receipt().hash(), decision.close().clone()));
                Ok(())
            },
            writer,
        )
    }

    fn update_epoch_owner_state_with_writer(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        apply: impl FnOnce(&mut EpochOwnerReceiptState) -> Result<(), AppError>,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
    ) -> Result<EpochOwnerReceiptState, AppError> {
        let scope = scope_bytes(server, document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        let (mut state, size) = match self.read_epoch_owner_record(&scope, document) {
            Ok(value) => value,
            Err(error) => {
                budget.invalidate();
                return Err(error);
            }
        };
        let observed = size
            .map(|size| storage_record(server, document, &scope, size))
            .transpose()?;
        budget
            .verify_record(&storage_scope, *blake3::hash(&scope).as_bytes(), observed)
            .map_err(invalid)?;
        apply(&mut state)?;
        let plain = state.encode(&scope, document)?;
        let record = storage_record(server, document, &scope, plain.len() as u64 + 40)?;
        let reservation = budget
            .reserve(
                &storage_scope,
                Replacement {
                    record,
                    scratch_bytes: 0,
                    purpose: WritePurpose::Settlement,
                },
            )
            .map_err(invalid)?;
        let sealed = match self.keys.db_key().and_then(|key| seal(&key, &plain, rng)) {
            Ok(sealed) => sealed,
            Err(error) => {
                reservation.cancel_before_write();
                return Err(error.into());
            }
        };
        writer(&self.epoch_owner_path(&scope), &frame(&sealed))?;
        reservation.commit();
        Ok(state)
    }

    fn epoch_owner_path(&self, scope: &[u8]) -> PathBuf {
        self.dir
            .join("servers")
            .join(format!("{}.owner-receipts", blake3::hash(scope).to_hex()))
    }

    fn read_epoch_owner_record(
        &self,
        scope: &[u8],
        document: &LogicalDocument,
    ) -> Result<(EpochOwnerReceiptState, Option<u64>), AppError> {
        let parent = self.dir.join("servers");
        let parent_meta = fs::symlink_metadata(&parent).map_err(|e| AppError::Io(e.to_string()))?;
        if !parent_meta.is_dir() || is_link(&parent_meta) {
            return Err(invalid("parent is not a regular directory"));
        }
        let path = self.epoch_owner_path(scope);
        match self.read_epoch_owner_plain(&path)? {
            None => Ok((EpochOwnerReceiptState::default(), None)),
            Some(bytes) => Ok((
                EpochOwnerReceiptState::decode(&bytes.plain, scope, document)?,
                Some(bytes.physical_bytes),
            )),
        }
    }

    // Same bounded reader for addressed loads and discovery; inventory must not weaken this
    // namespace's small cap or treat a disappeared enumerated record as an empty journal.
    pub(super) fn read_epoch_owner_plain(
        &self,
        path: &Path,
    ) -> Result<Option<AuthenticatedEpochFileBytes>, AppError> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(AppError::Io(e.to_string())),
        };
        if !regular_file(&metadata) || metadata.len() > MAX_SEALED_BYTES as u64 {
            return Err(invalid("journal is not a bounded regular file"));
        }
        let file = File::open(path).map_err(|e| AppError::Io(e.to_string()))?;
        if !regular_file(&file.metadata().map_err(|e| AppError::Io(e.to_string()))?) {
            return Err(invalid("opened journal is not regular"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_SEALED_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| AppError::Io(e.to_string()))?;
        if bytes.len() > MAX_SEALED_BYTES {
            return Err(invalid("journal exceeds its bound"));
        }
        let plain = Zeroizing::new(unseal(&self.keys.db_key()?, &unframe(&bytes)?)?);
        Ok(Some(AuthenticatedEpochFileBytes {
            plain,
            physical_bytes: bytes.len() as u64,
        }))
    }
}

pub(super) fn scope_bytes(server: u64, document: &LogicalDocument) -> Result<Vec<u8>, AppError> {
    // Bound before cloning public fields. The constructor below is the schema authority.
    if document.server_id.len() > 256 || document.logical_key.len() > 192 {
        return Err(invalid("scope exceeds its bound"));
    }
    LogicalDocument::new(
        document.server_id.clone(),
        document.doc_type,
        document.logical_key.clone(),
    )
    .map_err(invalid)?;
    let mut e = Encoder::new();
    e.put_bytes(RECORD_DOMAIN).expect("constant fits");
    e.put_u64(server);
    e.put_bytes(&document.server_id).map_err(invalid)?;
    e.put_u16(document.doc_type.tag());
    e.put_bytes(&document.logical_key).map_err(invalid)?;
    Ok(e.finish())
}

pub(super) fn storage_record(
    server: u64,
    document: &LogicalDocument,
    scope: &[u8],
    bytes: u64,
) -> Result<StorageRecord, AppError> {
    // Preserve the already-shipped recovery ownership derivation. The file id is namespaced,
    // but all files for one logical document MUST share the reserve's document owner key.
    let document_scope = super::epoch_recovery::scope_bytes(server, document)?;
    Ok(StorageRecord {
        id: *blake3::hash(scope).as_bytes(),
        document: *blake3::hash(&document_scope).as_bytes(),
        footprint: Footprint {
            protocol: bytes,
            ..Footprint::default()
        },
    })
}

fn invalid(error: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("epoch owner receipt: {error}"))
}

#[cfg(test)]
mod inventory_tests;

#[cfg(test)]
mod decision_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_mls::MlsDevice;
    use catcoms_replication::{InheritedCheckpoint, RecoveryReason, RecoverySnapshot};
    use catcoms_rt::ManualClock;
    use catcoms_wire::DocType;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    const SERVER: u64 = 7;

    fn rng() -> ChaCha20Rng {
        ChaCha20Rng::seed_from_u64(17)
    }

    fn open(path: &Path) -> ServerStore {
        ServerStore::open(path, b"owner-journal-test", &mut rng()).unwrap()
    }

    fn fixture() -> (MlsDevice, ServerGroup, LogicalDocument) {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let doc = LogicalDocument::new(
            group.group_id(),
            DocType::StudioObject,
            b"private-cat".to_vec(),
        )
        .unwrap();
        (owner, group, doc)
    }

    fn receipt(
        owner: &MlsDevice,
        group: &ServerGroup,
        doc: &LogicalDocument,
        epoch: u64,
    ) -> Receipt {
        Receipt::sign(
            doc.clone(),
            epoch,
            [epoch as u8; 32],
            [17; 32],
            group.epoch(),
            InheritedCheckpoint::EpochZero,
            owner,
        )
        .unwrap()
    }

    // Fixtures contain only the explicitly enumerated final records and no temporaries. This
    // helper is NOT a production inventory: namespace/orphan discovery remains integration work.
    fn budget(store: &ServerStore, doc: &LogicalDocument) -> EpochStorageBudget {
        let records = store
            .epoch_owner_receipt_inventory_record(SERVER, doc)
            .unwrap()
            .into_iter()
            .chain(store.epoch_recovery_inventory_record(SERVER, doc).unwrap());
        EpochStorageBudget::from_inventory(
            StorageScope::new(SERVER, &doc.server_id).unwrap(),
            records,
        )
        .unwrap()
    }

    fn prepare(
        store: &mut ServerStore,
        r: Receipt,
        group: &ServerGroup,
        budget: &mut EpochStorageBudget,
    ) -> EpochOwnerReceiptState {
        store
            .prepare_epoch_owner_receipt(SERVER, r, group, group.epoch(), &mut rng(), budget)
            .unwrap()
    }

    #[test]
    fn owner_close_provenance_survives_stale_completion_but_not_a_different_prepare() {
        let root = tempfile::tempdir().unwrap();
        let (owner, group, doc) = fixture();
        let mut store = open(root.path());
        let mut budget = budget(&store, &doc);
        let decisions: Vec<_> = (0..2)
            .map(|epoch| {
                let close =
                    CloseRecord::sign(&doc, 4, epoch, vec![[epoch as u8; 32]], &owner).unwrap();
                let receipt = Receipt::sign(
                    doc.clone(),
                    epoch,
                    close.hash(),
                    [17; 32],
                    group.epoch(),
                    InheritedCheckpoint::EpochZero,
                    &owner,
                )
                .unwrap();
                (receipt, close)
            })
            .collect();
        for (receipt, close) in &decisions {
            // Install an authenticated close-bearing fixture through the real accounted writer.
            // Closure eligibility belongs to the core builder tests; this regression targets the
            // generic public prepare/completion wrapper's preservation and replacement rules.
            store
                .update_epoch_owner_state_with_writer(
                    SERVER,
                    &doc,
                    &mut rng(),
                    &mut budget,
                    |state| {
                        state
                            .journal
                            .prepare(receipt.clone(), &group, group.epoch())
                            .map_err(invalid)?;
                        state.decision_close = Some((receipt.hash(), close.clone()));
                        Ok(())
                    },
                    atomic_write,
                )
                .unwrap();
            // Repeating completion of epoch zero while epoch one is pending must preserve the
            // selected epoch-one close, not clear it or reattach the published epoch-zero close.
            store
                .mark_epoch_owner_receipt_published(
                    SERVER,
                    &doc,
                    decisions[0].0.hash(),
                    &mut rng(),
                    &mut budget,
                )
                .unwrap();
            drop(store);
            store = open(root.path());
            budget = super::tests::budget(&store, &doc);
            let saved = store.load_epoch_owner_receipts(SERVER, &doc).unwrap();
            assert_eq!(saved.published(), Some(&decisions[0].0));
            assert_eq!(saved.close_for(receipt).unwrap().encode(), close.encode());
            assert_eq!(
                saved.pending(),
                (receipt.closed_epoch == 1).then_some(receipt)
            );
        }
        let (second, _) = &decisions[1];
        store
            .mark_epoch_owner_receipt_published(
                SERVER,
                &doc,
                second.hash(),
                &mut rng(),
                &mut budget,
            )
            .unwrap();
        let third = receipt(&owner, &group, &doc, 2);
        prepare(&mut store, third.clone(), &group, &mut budget);
        drop(store);
        let saved = open(root.path())
            .load_epoch_owner_receipts(SERVER, &doc)
            .unwrap();
        assert_eq!(saved.pending(), Some(&third));
        assert_eq!(saved.published(), Some(second));
        assert!(saved.close_for(second).is_none());
        assert!(saved.close_for(&third).is_none());
        assert!(saved.decision_close.is_none());
    }

    #[test]
    fn owner_decision_survives_reopen_and_exact_completion_retry_preserves_next_pending() {
        let root = tempfile::tempdir().unwrap();
        let (owner, group, doc) = fixture();
        let first = receipt(&owner, &group, &doc, 0);
        let mut store = open(root.path());
        let mut budget = budget(&store, &doc);
        assert!(store
            .load_epoch_owner_receipts(SERVER, &doc)
            .unwrap()
            .pending()
            .is_none());
        let state = prepare(&mut store, first.clone(), &group, &mut budget);
        assert_eq!(state.pending(), Some(&first));
        assert!(!format!("{state:?}").contains("private-cat"));
        let path = store.epoch_owner_path(&scope_bytes(SERVER, &doc).unwrap());
        assert!(!fs::read(&path)
            .unwrap()
            .windows(b"private-cat".len())
            .any(|w| w == b"private-cat"));
        let before = fs::read(&path).unwrap();
        let mut conflict = first.clone();
        conflict.close_record_hash = [99; 32]; // Re-sign a valid competing choice.
        conflict = Receipt::sign(
            doc.clone(),
            0,
            conflict.close_record_hash,
            [17; 32],
            group.epoch(),
            InheritedCheckpoint::EpochZero,
            &owner,
        )
        .unwrap();
        assert!(store
            .prepare_epoch_owner_receipt(
                SERVER,
                conflict,
                &group,
                group.epoch(),
                &mut rng(),
                &mut budget
            )
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        drop(store);
        let mut store = open(root.path());
        let mut budget = super::tests::budget(&store, &doc);
        assert_eq!(
            store
                .load_epoch_owner_receipts(SERVER, &doc)
                .unwrap()
                .pending(),
            Some(&first)
        );
        prepare(&mut store, first.clone(), &group, &mut budget);
        store
            .mark_epoch_owner_receipt_published(SERVER, &doc, first.hash(), &mut rng(), &mut budget)
            .unwrap();
        let next = receipt(&owner, &group, &doc, 1);
        prepare(&mut store, next.clone(), &group, &mut budget);
        // Preparation must resume the pending choice, not republish an older head. Completion
        // retries below acknowledge old work without changing the new work.
        assert!(store
            .prepare_epoch_owner_with_writer(
                SERVER,
                first.clone(),
                &group,
                group.epoch(),
                &mut rng(),
                &mut budget,
                |_, _| panic!("stale prepare wrote")
            )
            .is_err());
        let retry = store
            .mark_epoch_owner_receipt_published(SERVER, &doc, first.hash(), &mut rng(), &mut budget)
            .unwrap();
        assert_eq!(retry.pending(), Some(&next));
        assert_eq!(retry.published(), Some(&first));
        assert!(store
            .mark_epoch_owner_receipt_published(SERVER, &doc, [99; 32], &mut rng(), &mut budget)
            .is_err());
        assert_eq!(
            store
                .load_epoch_owner_receipts(SERVER, &doc)
                .unwrap()
                .pending(),
            Some(&next)
        );
    }

    #[test]
    fn preparation_and_completion_flush_failures_return_no_success_and_retry_exactly() {
        for completing in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (owner, group, doc) = fixture();
            let signed = receipt(&owner, &group, &doc, 0);
            let mut store = open(root.path());
            let mut budget = budget(&store, &doc);
            if completing {
                prepare(&mut store, signed.clone(), &group, &mut budget);
            }
            let writer = |path: &Path, bytes: &[u8]| {
                atomic_write_with_hook_and_sync(
                    path,
                    bytes,
                    |_, _| {},
                    |_| Err(std::io::Error::other("flush failure")),
                )
            };
            let result = if completing {
                store.update_epoch_owner_with_writer(
                    SERVER,
                    &doc,
                    &mut rng(),
                    &mut budget,
                    |j| j.mark_published(signed.hash()).map_err(invalid),
                    writer,
                )
            } else {
                store.prepare_epoch_owner_with_writer(
                    SERVER,
                    signed.clone(),
                    &group,
                    group.epoch(),
                    &mut rng(),
                    &mut budget,
                    writer,
                )
            };
            assert!(matches!(result, Err(AppError::CommittedButNotDurable(_))));
            assert!(budget.requires_reconciliation());
            assert!(store
                .prepare_epoch_owner_receipt(
                    SERVER,
                    signed.clone(),
                    &group,
                    group.epoch(),
                    &mut rng(),
                    &mut budget
                )
                .is_err());
            drop(store);
            let mut store = open(root.path());
            let mut budget = super::tests::budget(&store, &doc); // Rename consumed the temporary.
            let result = if completing {
                store
                    .mark_epoch_owner_receipt_published(
                        SERVER,
                        &doc,
                        signed.hash(),
                        &mut rng(),
                        &mut budget,
                    )
                    .unwrap()
            } else {
                prepare(&mut store, signed.clone(), &group, &mut budget)
            };
            assert_eq!(result.pending().or(result.published()), Some(&signed));
            assert!(!budget.requires_reconciliation());
        }
    }

    #[test]
    fn failures_before_rename_and_caught_panics_preserve_the_old_decision_and_block_budget() {
        for panic in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (owner, group, doc) = fixture();
            let signed = receipt(&owner, &group, &doc, 0);
            let mut store = open(root.path());
            let mut budget = budget(&store, &doc);
            prepare(&mut store, signed.clone(), &group, &mut budget);
            let path = store.epoch_owner_path(&scope_bytes(SERVER, &doc).unwrap());
            let before = fs::read(&path).unwrap();
            let orphan = staging_candidate(&path, 901);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                store.update_epoch_owner_with_writer(
                    SERVER,
                    &doc,
                    &mut rng(),
                    &mut budget,
                    |j| j.mark_published(signed.hash()).map_err(invalid),
                    |_, bytes| {
                        fs::write(&orphan, bytes).unwrap();
                        if panic {
                            panic!("injected after temporary write");
                        }
                        Err(AppError::Io("injected before rename".into()))
                    },
                )
            }));
            assert!(if panic {
                result.is_err()
            } else {
                result.unwrap().is_err()
            });
            assert!(budget.requires_reconciliation());
            assert_eq!(fs::read(&path).unwrap(), before);
            assert!(orphan.exists());
            drop(store);
            let store = open(root.path());
            assert_eq!(
                store
                    .load_epoch_owner_receipts(SERVER, &doc)
                    .unwrap()
                    .pending(),
                Some(&signed)
            );
            // Orphan is neither loaded nor treated as publication; no partial inventory refund.
            assert!(orphan.exists());
        }
    }

    #[test]
    fn accounting_shares_recovery_reserve_owner_but_charges_journal_to_protocol() {
        let root = tempfile::tempdir().unwrap();
        let (owner, group, doc) = fixture();
        let mut store = open(root.path());
        for epoch in 0..3 {
            store
                .update_epoch_recovery(
                    SERVER,
                    &doc,
                    EpochRecoveryAction::Stage(RecoverySnapshot {
                        doc_type: doc.doc_type,
                        logical_key: doc.logical_key.clone(),
                        epoch,
                        base_close_record_hash: None,
                        reason: RecoveryReason::Excluded,
                        projection: vec![epoch as u8],
                        tombstones: vec![],
                        elements: vec![],
                        conflicts: vec![],
                        applied_ops: vec![],
                    }),
                    &ManualClock::new(0),
                    &mut rng(),
                )
                .unwrap();
        }
        let recovery = store
            .epoch_recovery_inventory_record(SERVER, &doc)
            .unwrap()
            .unwrap();
        assert!(recovery.footprint.settlement > 0);
        let mut budget = budget(&store, &doc);
        let before = budget.usage();
        prepare(
            &mut store,
            receipt(&owner, &group, &doc, 0),
            &group,
            &mut budget,
        );
        let saved = store
            .epoch_owner_receipt_inventory_record(SERVER, &doc)
            .unwrap()
            .unwrap();
        assert_ne!(saved.id, recovery.id);
        assert_eq!(saved.document, recovery.document);
        assert_eq!(budget.usage().content, before.content);
        assert_eq!(budget.usage().settlement, before.settlement);
        assert_eq!(budget.usage().protocol, saved.footprint.protocol);
        assert_eq!(
            saved.footprint.total().unwrap(),
            fs::metadata(store.epoch_owner_path(&scope_bytes(SERVER, &doc).unwrap()))
                .unwrap()
                .len()
        );
    }

    #[test]
    fn budget_or_authority_refusal_happens_before_the_writer() {
        let root = tempfile::tempdir().unwrap();
        let (owner, group, doc) = fixture();
        let mut store = open(root.path());
        let scope = StorageScope::new(SERVER, &doc.server_id).unwrap();
        let mut budget = EpochStorageBudget::from_inventory(
            scope,
            [StorageRecord {
                id: [90; 32],
                document: [91; 32],
                footprint: Footprint {
                    protocol: super::super::epoch_budget::PROTOCOL_ALLOWANCE_BYTES,
                    ..Footprint::default()
                },
            }],
        )
        .unwrap();
        let result = store.prepare_epoch_owner_with_writer(
            SERVER,
            receipt(&owner, &group, &doc, 0),
            &group,
            group.epoch(),
            &mut rng(),
            &mut budget,
            |_, _| panic!("over-cap write"),
        );
        assert!(result.is_err());
        assert!(!budget.requires_reconciliation());
        assert!(store
            .load_epoch_owner_receipts(SERVER, &doc)
            .unwrap()
            .pending()
            .is_none());
        let mut budget = super::tests::budget(&store, &doc);
        let outsider = MlsDevice::generate().unwrap();
        let bad = receipt(&outsider, &group, &doc, 0);
        assert!(store
            .prepare_epoch_owner_with_writer(
                SERVER,
                bad,
                &group,
                group.epoch(),
                &mut rng(),
                &mut budget,
                |_, _| panic!("unauthorized write")
            )
            .is_err());
        let mut malformed = receipt(&owner, &group, &doc, 0);
        malformed.document.logical_key = vec![0; 193];
        assert!(store
            .prepare_epoch_owner_receipt(
                SERVER,
                malformed,
                &group,
                group.epoch(),
                &mut rng(),
                &mut budget
            )
            .is_err());
        assert!(!budget.requires_reconciliation());
    }

    #[test]
    fn returning_owner_replaces_old_tenure_pending_only_after_a_new_durable_decision() {
        let root = tempfile::tempdir().unwrap();
        let alice = MlsDevice::generate().unwrap();
        // Save a second provider before the first group exists: rejoining must not try to
        // install the same GroupId into Alice's old provider (which still retains that group).
        let returning_alice = alice.duplicate().unwrap();
        let mut alice_group = ServerGroup::create(&alice).unwrap();
        let doc = LogicalDocument::new(
            alice_group.group_id(),
            DocType::StudioObject,
            b"private-cat".to_vec(),
        )
        .unwrap();
        let old = receipt(&alice, &alice_group, &doc, 0);
        let mut store = open(root.path());
        let mut budget = budget(&store, &doc);
        prepare(&mut store, old.clone(), &alice_group, &mut budget);
        drop(store);

        // Exercise actual MLS roster/epoch verification. Raw group operations build the A-B-A
        // fixture; application authorization of membership changes is a separately tested path.
        let bob = MlsDevice::generate().unwrap();
        let welcome = alice_group
            .add_member(&alice, bob.key_package().unwrap())
            .unwrap()
            .welcome;
        let mut bob_group = ServerGroup::join(&bob, &welcome).unwrap();
        bob_group.remove_member(&bob, &alice.device_id()).unwrap();
        assert_eq!(bob_group.designated_committer(), Some(bob.device_id()));
        let welcome = bob_group
            .add_member(&bob, returning_alice.key_package().unwrap())
            .unwrap()
            .welcome;
        let alice_group = ServerGroup::join(&returning_alice, &welcome).unwrap();
        assert_eq!(alice_group.designated_committer(), Some(alice.device_id()));
        let new = Receipt::sign(
            doc.clone(),
            3,
            [33; 32],
            [34; 32],
            alice_group.epoch(),
            InheritedCheckpoint::Checkpoint {
                epoch: 3,
                close_record_hash: [31; 32],
                seed_change_hash: [32; 32],
            },
            &alice,
        )
        .unwrap();
        let mut store = open(root.path());
        let mut budget = super::tests::budget(&store, &doc);
        let invalid_first = Receipt::sign(
            doc.clone(),
            4,
            [40; 32],
            [41; 32],
            alice_group.epoch(),
            new.inherited.clone(),
            &alice,
        )
        .unwrap();
        let path = store.epoch_owner_path(&scope_bytes(SERVER, &doc).unwrap());
        let before = fs::read(&path).unwrap();
        // A later authenticated tenure is not sufficient: its FIRST receipt must close the
        // inherited epoch. Refusal cannot discard the previous tenure's pending decision.
        assert!(store
            .prepare_epoch_owner_with_writer(
                SERVER,
                invalid_first,
                &alice_group,
                alice_group.epoch(),
                &mut rng(),
                &mut budget,
                |_, _| panic!("invalid adoption wrote")
            )
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(store
            .prepare_epoch_owner_receipt(
                SERVER,
                old.clone(),
                &alice_group,
                alice_group.epoch(),
                &mut rng(),
                &mut budget
            )
            .is_err());
        assert_eq!(
            store
                .load_epoch_owner_receipts(SERVER, &doc)
                .unwrap()
                .pending(),
            Some(&old)
        );
        let failure = store.prepare_epoch_owner_with_writer(
            SERVER,
            new.clone(),
            &alice_group,
            alice_group.epoch(),
            &mut rng(),
            &mut budget,
            |_, _| Err(AppError::Io("before write".into())),
        );
        assert!(failure.is_err());
        assert_eq!(
            store
                .load_epoch_owner_receipts(SERVER, &doc)
                .unwrap()
                .pending(),
            Some(&old)
        );
        let mut budget = super::tests::budget(&store, &doc); // The injected writer wrote nothing.
        let saved = prepare(&mut store, new.clone(), &alice_group, &mut budget);
        assert_eq!(saved.pending(), Some(&new));
        assert!(saved.published().is_none());
        assert!(store
            .mark_epoch_owner_receipt_published(SERVER, &doc, old.hash(), &mut rng(), &mut budget)
            .is_err());
        drop(store);
        assert_eq!(
            open(root.path())
                .load_epoch_owner_receipts(SERVER, &doc)
                .unwrap()
                .pending(),
            Some(&new)
        );
    }

    #[test]
    fn repeated_rotations_remain_bounded_and_canonical_field_failures_do_not_write() {
        let root = tempfile::tempdir().unwrap();
        let (owner, group, doc) = fixture();
        let mut store = open(root.path());
        let mut budget = budget(&store, &doc);
        let mut settled_length = None;
        for epoch in 0..12 {
            let signed = receipt(&owner, &group, &doc, epoch);
            prepare(&mut store, signed.clone(), &group, &mut budget);
            store
                .mark_epoch_owner_receipt_published(
                    SERVER,
                    &doc,
                    signed.hash(),
                    &mut rng(),
                    &mut budget,
                )
                .unwrap();
            let length = store
                .epoch_owner_receipt_inventory_record(SERVER, &doc)
                .unwrap()
                .unwrap()
                .footprint
                .protocol;
            assert_eq!(*settled_length.get_or_insert(length), length);
            assert!(length <= MAX_SEALED_BYTES as u64);
        }
        // All fields can be populated through a Rust struct literal. This shape can be signed,
        // but canonical receipt decoding rejects Checkpoint{epoch:0}; it must not reach disk.
        let malformed = Receipt::sign(
            doc.clone(),
            12,
            [44; 32],
            [45; 32],
            group.epoch(),
            InheritedCheckpoint::Checkpoint {
                epoch: 0,
                close_record_hash: [46; 32],
                seed_change_hash: [47; 32],
            },
            &owner,
        )
        .unwrap();
        assert!(store
            .prepare_epoch_owner_with_writer(
                SERVER,
                malformed,
                &group,
                group.epoch(),
                &mut rng(),
                &mut budget,
                |_, _| panic!("noncanonical write")
            )
            .is_err());
        assert_eq!(
            store
                .load_epoch_owner_receipts(SERVER, &doc)
                .unwrap()
                .published()
                .unwrap()
                .closed_epoch,
            11
        );
    }

    #[test]
    fn copied_corrupt_oversized_and_nonregular_journals_never_reset_a_decision() {
        let root = tempfile::tempdir().unwrap();
        let (owner, group, doc) = fixture();
        let mut store = open(root.path());
        let mut budget = budget(&store, &doc);
        prepare(
            &mut store,
            receipt(&owner, &group, &doc, 0),
            &group,
            &mut budget,
        );
        let path = store.epoch_owner_path(&scope_bytes(SERVER, &doc).unwrap());
        let valid = fs::read(&path).unwrap();
        for (server, altered) in [
            (8, doc.clone()),
            (
                SERVER,
                LogicalDocument {
                    doc_type: DocType::PostReplies,
                    ..doc.clone()
                },
            ),
            (
                SERVER,
                LogicalDocument {
                    server_id: b"other-group".to_vec(),
                    ..doc.clone()
                },
            ),
        ] {
            let other_path = store.epoch_owner_path(&scope_bytes(server, &altered).unwrap());
            fs::write(other_path, &valid).unwrap();
            assert!(store.load_epoch_owner_receipts(server, &altered).is_err());
        }
        for data in [vec![0; 50], vec![0; MAX_SEALED_BYTES + 1]] {
            fs::write(&path, data).unwrap();
            assert!(store.load_epoch_owner_receipts(SERVER, &doc).is_err());
            assert!(store
                .prepare_epoch_owner_receipt(
                    SERVER,
                    receipt(&owner, &group, &doc, 0),
                    &group,
                    group.epoch(),
                    &mut rng(),
                    &mut budget
                )
                .is_err());
            assert!(budget.requires_reconciliation());
        }
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(store.load_epoch_owner_receipts(SERVER, &doc).is_err());
    }
}
