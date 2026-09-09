//! Accounted, vault-sealed Index/art Save/Load. This adapter owns both durability barriers;
//! it returns prepared ciphertext, never a network-send permit or proof of owner settlement.

use super::epoch_budget::{
    EpochStorageBudget, Footprint, Replacement, StorageRecord, StorageScope, WritePurpose,
};
use super::epoch_recovery::inventory::{is_link, regular_file};
use super::epoch_recovery::AuthenticatedEpochFileBytes;
use super::*;
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_replication::epoch::{MAX_RECEIPT_BYTES, MAX_SIGNED_EPOCH_OP_BYTES};
use catcoms_replication::studio::{
    FlipnoteOp, IndexOp, StudioEpoch, StudioProjection, StudioTarget,
    MAX_STUDIO_EPOCH_SNAPSHOT_BYTES,
};
use catcoms_replication::{
    Admission, DomainOp, EpochPhase, LogicalDocument, Receipt, ReceiptIngest, SealedOp,
};
use std::io::Read;
use std::sync::Arc;

pub(super) const RECORD_DOMAIN: &[u8] = b"catcoms/epoch-studio-store/v1";
const MAX_RECORD_BYTES: usize = MAX_STUDIO_EPOCH_SNAPSHOT_BYTES + 1024;
pub(super) const MAX_SEALED_BYTES: usize = MAX_RECORD_BYTES + 40;
const MAX_INBOUND_CIPHERTEXT: usize = MAX_SIGNED_EPOCH_OP_BYTES + 4 + 16;

// The detached source, its observed physical ledger entry (including actual absence), and
// normalized restart bytes must travel together across the intent/source durability barriers.
type CheckedStudioSource = (StudioEpoch, Option<StorageRecord>, Zeroizing<Vec<u8>>);

/// Five-family coverage plus existing per-server/storage and vault-intent budgets. This is not
/// a new budget algorithm. The caller must own the SOLE coordinator and forbid interleaved raw
/// registry/recovery/owner writes; blobs and legacy snapshots are not covered by this inventory.
/// A mount-local generation prevents duplicate wrappers/captured scans spending Studio bytes.
pub struct EpochStudioBudget {
    scope: StorageScope,
    generation: Arc<()>,
    storage: EpochStorageBudget,
    intents: EpochIntentBudget,
}
impl std::fmt::Debug for EpochStudioBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochStudioBudget")
            .field("storage", &self.storage)
            .finish_non_exhaustive()
    }
}
impl EpochStudioBudget {
    /// Accounted physical P1 bytes for this server, excluding blobs/legacy snapshot families.
    pub fn usage(&self) -> Footprint {
        self.storage.usage()
    }
    /// Reports uncertain accounting, not whether a newer mint superseded this wrapper. Every
    /// mutation independently checks the store's live generation before spending any bytes.
    pub fn requires_reconciliation(&self) -> bool {
        self.storage.requires_reconciliation()
    }
}
/// Detached read-only persisted state. No mutable document/gate/book escapes the store.
#[derive(Debug)]
pub struct EpochStudioState {
    unit: StudioEpoch,
}
impl EpochStudioState {
    /// Complete retained signed-envelope comparison; never use projection/marker equality to
    /// infer a retry. This read-only evidence does not replace the store's edit/flush barriers.
    pub fn contains_exact_operation(
        &self,
        author: catcoms_crypto::DeviceId,
        operation: &DomainOp,
    ) -> Result<bool, AppError> {
        self.unit
            .contains_exact_operation(author, operation)
            .map_err(invalid)
    }
    pub fn doc_id(&self) -> u128 {
        self.unit.doc_id()
    }
    pub fn epoch(&self) -> u64 {
        self.unit.epoch()
    }
    pub fn phase(&self) -> EpochPhase {
        self.unit.phase()
    }
    pub fn op_count(&self) -> usize {
        self.unit.op_count()
    }
    pub fn quarantined_len(&self) -> usize {
        self.unit.quarantined_len()
    }
    pub fn projection(&self) -> Result<StudioProjection, AppError> {
        self.unit.projection().map_err(invalid)
    }
}
impl ServerStore {
    /// Mint once from a completed CURRENT five-family scan. Minting again requires a new scan
    /// and supersedes the previous wrapper; reopening the vault invalidates all old handles.
    pub fn studio_storage_budget(
        &mut self,
        server: u64,
        group: &ServerGroup,
        inventory: &EpochStorageInventory,
    ) -> Result<EpochStudioBudget, AppError> {
        if inventory.coverage()
            != EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsRegistryAndStudio
            || !Arc::ptr_eq(&inventory.studio_generation, &self.studio_generation)
            || !Arc::ptr_eq(&inventory.intent_generation, &self.intent_generation)
        {
            return Err(invalid("fresh five-family inventory required"));
        }
        let scope = StorageScope::new(server, &group.group_id()).map_err(invalid)?;
        let storage = EpochStorageBudget::from_inventory(
            scope.clone(),
            inventory.records_for_server(server, &group.group_id())?,
        )
        .map_err(invalid)?;
        let intents = EpochIntentBudget::from_inventory(inventory)?;
        self.studio_generation = Arc::new(());
        Ok(EpochStudioBudget {
            scope,
            generation: self.studio_generation.clone(),
            storage,
            intents,
        })
    }
    fn enter_studio_budget(
        &mut self,
        server: u64,
        group: &ServerGroup,
        budget: &mut EpochStudioBudget,
    ) -> Result<(), AppError> {
        if budget.scope != StorageScope::new(server, &group.group_id()).map_err(invalid)?
            || !Arc::ptr_eq(&budget.generation, &self.studio_generation)
        {
            budget.storage.invalidate();
            return Err(invalid(
                "Studio budget is stale or belongs to another mount/server",
            ));
        }
        if budget.storage.requires_reconciliation() {
            return Err(invalid("Studio inventory requires reconciliation"));
        }
        // Even unchanged-file flush attempts invalidate captured inventories. The active owner
        // keeps its token; failed reservations independently require a fresh disk reconciliation.
        self.studio_generation = Arc::new(());
        budget.generation = self.studio_generation.clone();
        Ok(())
    }
    /// Bounded read-only vault restore. Only actual absence returns None; malformed/incoherent
    /// state is an error, never an invitation to overwrite it with a fresh epoch-zero document.
    pub fn load_studio_epoch(
        &self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
    ) -> Result<Option<EpochStudioState>, AppError> {
        let logical = target.document(&group.group_id()).map_err(invalid)?;
        let scope = scope_bytes(server, &logical)?;
        self.read_studio_record(&scope)?
            .map(|bytes| {
                let (stored, snapshot) = decode_record(&bytes.plain, &scope, &logical)?;
                if stored != target {
                    return Err(invalid("wrong object channel"));
                }
                Ok(EpochStudioState {
                    unit: StudioEpoch::restore(snapshot, group, target, device.device_id())
                        .map_err(invalid)?,
                })
            })
            .transpose()
    }
    /// Preserve nonce/envelope and concrete expected_doc_id across retry. This saves the intent
    /// before the epoch and only then returns ciphertext. Reference holds precede persistence;
    /// this does not publish, fetch, or establish that the referenced bytes are present.
    #[allow(clippy::too_many_arguments)]
    pub fn edit_studio_epoch(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        expected_doc_id: u128,
        device: &MlsDevice,
        operation: DomainOp,
        ts: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<(SealedOp, EpochStudioState), AppError> {
        self.edit_studio_with_io(
            server,
            group,
            target,
            expected_doc_id,
            device,
            operation,
            ts,
            rng,
            budget,
            atomic_write,
            super::epoch_intents::sync_intent,
            atomic_write,
            sync_studio,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn edit_studio_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        expected_doc_id: u128,
        device: &MlsDevice,
        operation: DomainOp,
        ts: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        intent_writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        intent_sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
        epoch_writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        epoch_sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<(SealedOp, EpochStudioState), AppError> {
        let logical = target.document(&group.group_id()).map_err(invalid)?;
        match target {
            StudioTarget::Index { .. } => {
                IndexOp::decode_domain(&logical, &operation, &device.device_id())
                    .map_err(invalid)?;
            }
            StudioTarget::Flipnote { .. } => {
                FlipnoteOp::decode_domain(&logical, &operation).map_err(invalid)?;
            }
        }
        current_member(group, device)?;
        self.enter_studio_budget(server, group, budget)?;
        // Verify observed presence OR absence before the intent barrier. A lost indexed epoch
        // cannot silently become a fresh epoch-zero document with a newly stranded intent.
        let (mut unit, observed, before) =
            self.checked_studio_source(server, group, target, device, true, &mut budget.storage)?;
        if unit.doc_id() != expected_doc_id {
            return Err(invalid("edit belongs to a retired epoch"));
        }
        unit.validate_local_edit(device, group, &operation, ts)
            .map_err(invalid)?;
        self.prepare_epoch_intent_with_io(
            server,
            &logical,
            operation.clone(),
            device,
            group,
            rng,
            &mut budget.storage,
            &mut budget.intents,
            intent_writer,
            intent_sync,
        )?;
        // The same exclusive store borrow retains this checked detached source across both
        // barriers. Intent persistence cannot mutate the Studio source. Failure keeps the intent.
        let sealed = unit
            .edit_or_reseal(device, group, rng, &operation, ts)
            .map_err(invalid)?;
        let state = self.save_studio_source(
            server,
            unit,
            observed,
            &before,
            WritePurpose::Ordinary,
            rng,
            &mut budget.storage,
            epoch_writer,
            epoch_sync,
        )?;
        Ok((sealed, state))
    }
    /// Saved Duplicate/Late results cross a flush barrier too. Late is quarantine, not edit ack.
    #[allow(clippy::too_many_arguments)]
    pub fn ingest_studio_epoch(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        sealed: &SealedOp,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<(Admission, EpochStudioState), AppError> {
        if sealed.blob.ciphertext.len() > MAX_INBOUND_CIPHERTEXT {
            return Err(invalid("inbound ciphertext too large"));
        }
        current_member(group, device)?;
        self.enter_studio_budget(server, group, budget)?;
        let (mut unit, observed, before) =
            self.checked_studio_source(server, group, target, device, true, &mut budget.storage)?;
        let outcome = unit.ingest(sealed, group, device).map_err(invalid)?;
        let state = self.save_studio_source(
            server,
            unit,
            observed,
            &before,
            WritePurpose::Ordinary,
            rng,
            &mut budget.storage,
            atomic_write,
            sync_studio,
        )?;
        Ok((outcome, state))
    }
    /// Persist receipt/book/gate together, including a typed Fault outcome, with full source
    /// history retained. This does NOT produce receipts, settle/replace epochs, or prune anything.
    #[allow(clippy::too_many_arguments)]
    pub fn seal_studio_epoch(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        receipt: Receipt,
        tenure_start: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<(ReceiptIngest, EpochStudioState), AppError> {
        self.seal_studio_with_io(
            server,
            group,
            target,
            device,
            receipt,
            tenure_start,
            rng,
            budget,
            atomic_write,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn seal_studio_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        receipt: Receipt,
        tenure_start: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
    ) -> Result<(ReceiptIngest, EpochStudioState), AppError> {
        let logical = target.document(&group.group_id()).map_err(invalid)?;
        scope_bytes(server, &receipt.document)?;
        if receipt.document != logical
            || receipt.owner_public_key.len() != 32
            || receipt.encode().len() > MAX_RECEIPT_BYTES
        {
            return Err(invalid("receipt exceeds scope/bounds"));
        }
        current_member(group, device)?;
        // Reject forged/stale authority before expensive signed-log reconstruction or changing
        // inventory generations. The owned unit rechecks under its gate before mutation.
        receipt
            .verify_current_owner(group, tenure_start)
            .map_err(invalid)?;
        self.enter_studio_budget(server, group, budget)?;
        let (mut unit, observed, before) =
            self.checked_studio_source(server, group, target, device, false, &mut budget.storage)?;
        let outcome = unit.seal(receipt, group, tenure_start).map_err(invalid)?;
        let state = self.save_studio_source(
            server,
            unit,
            observed,
            &before,
            WritePurpose::Settlement,
            rng,
            &mut budget.storage,
            writer,
            sync_studio,
        )?;
        Ok((outcome, state))
    }
    #[allow(clippy::too_many_arguments)]
    fn checked_studio_source(
        &self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        allow_create: bool,
        budget: &mut EpochStorageBudget,
    ) -> Result<CheckedStudioSource, AppError> {
        let logical = target.document(&group.group_id()).map_err(invalid)?;
        let scope = scope_bytes(server, &logical)?;
        let loaded = (|| {
            let held = self.read_studio_record(&scope)?;
            let mut unit = match &held {
                Some(bytes) => {
                    let (stored, snapshot) = decode_record(&bytes.plain, &scope, &logical)?;
                    if stored != target {
                        return Err(invalid("wrong object channel"));
                    }
                    StudioEpoch::restore(snapshot, group, target, device.device_id())
                        .map_err(invalid)?
                }
                None => StudioEpoch::new(group, target, device.device_id()).map_err(invalid)?,
            };
            let observed = held
                .as_ref()
                .map(|b| {
                    storage_record(
                        server,
                        &logical,
                        &scope,
                        b.physical_bytes,
                        unit.storage_protocol_bytes().map_err(invalid)?,
                    )
                })
                .transpose()?;
            let before = Zeroizing::new(unit.snapshot().map_err(invalid)?);
            Ok((unit, observed, before))
        })();
        let (unit, observed, before) = loaded.inspect_err(|_| {
            budget.invalidate();
        })?;
        budget
            .verify_record(
                &StorageScope::new(server, &logical.server_id).map_err(invalid)?,
                *blake3::hash(&scope).as_bytes(),
                observed,
            )
            .map_err(invalid)?;
        if observed.is_none() && !allow_create {
            return Err(invalid("source missing; fetch before sealing"));
        }
        Ok((unit, observed, before))
    }
    #[allow(clippy::too_many_arguments)]
    fn save_studio_source(
        &self,
        server: u64,
        mut unit: StudioEpoch,
        observed: Option<StorageRecord>,
        before: &[u8],
        purpose: WritePurpose,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<EpochStudioState, AppError> {
        let scope = scope_bytes(server, unit.document())?;
        // Add before either write/flush barrier, and keep holds even when persistence is uncertain.
        self.hold_creative(
            &unit.document().server_id,
            unit.blob_cids().map_err(invalid),
        );
        let storage_scope =
            StorageScope::new(server, &unit.document().server_id).map_err(invalid)?;
        let snapshot = Zeroizing::new(unit.snapshot().map_err(invalid)?);
        let path = self.studio_epoch_path(&scope);
        if let Some(record) = observed.filter(|_| before == snapshot.as_slice()) {
            let reservation = budget
                .reserve_sync(&storage_scope, record)
                .map_err(invalid)?;
            sync(&path, record.footprint.total().map_err(invalid)?)?;
            reservation.commit();
        } else {
            let mut e = Encoder::new();
            e.put_bytes(&scope).map_err(invalid)?;
            e.put_bytes(&unit.target().channel()).map_err(invalid)?;
            e.put_bytes(&snapshot).map_err(invalid)?;
            let plain = Zeroizing::new(e.finish());
            let record = storage_record(
                server,
                unit.document(),
                &scope,
                plain.len() as u64 + 40,
                unit.storage_protocol_bytes().map_err(invalid)?,
            )?;
            let reservation = budget
                .reserve(
                    &storage_scope,
                    Replacement {
                        record,
                        scratch_bytes: 0,
                        purpose,
                    },
                )
                .map_err(invalid)?;
            let sealed = match self.keys.db_key().and_then(|key| seal(&key, &plain, rng)) {
                Ok(value) => value,
                Err(error) => {
                    reservation.cancel_before_write();
                    return Err(error.into());
                }
            };
            writer(&path, &frame(&sealed))?;
            reservation.commit();
        }
        Ok(EpochStudioState { unit })
    }
    fn studio_epoch_path(&self, scope: &[u8]) -> PathBuf {
        self.dir
            .join("servers")
            .join(format!("{}.studio-epoch", blake3::hash(scope).to_hex()))
    }
    /// A local background-work refusal only. Metadata can refuse expensive target restoration,
    /// never authorize it: the later ingest still authenticates and validates the actual source.
    pub(crate) fn check_studio_receive_source_bound(
        &self,
        server: u64,
        group: &[u8],
        target: StudioTarget,
    ) -> Result<(), AppError> {
        let logical = target.document(group).map_err(invalid)?;
        let scope = scope_bytes(server, &logical)?;
        let parent = fs::symlink_metadata(self.dir.join("servers"))
            .map_err(|e| AppError::Io(e.to_string()))?;
        if !parent.is_dir() || is_link(&parent) {
            return Err(invalid("parent is not a regular directory"));
        }
        let metadata = match fs::symlink_metadata(self.studio_epoch_path(&scope)) {
            Ok(value) => value,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(AppError::Io(e.to_string())),
        };
        if !regular_file(&metadata)
            || metadata.len() > super::epoch_recovery::inventory::STUDIO_RECEIVE_COLD_BYTES
        {
            return Err(invalid(
                "automatic Studio target source exceeds cold byte limit or is not regular",
            ));
        }
        Ok(())
    }
    fn read_studio_record(
        &self,
        scope: &[u8],
    ) -> Result<Option<AuthenticatedEpochFileBytes>, AppError> {
        let metadata = fs::symlink_metadata(self.dir.join("servers"))
            .map_err(|e| AppError::Io(e.to_string()))?;
        if !metadata.is_dir() || is_link(&metadata) {
            return Err(invalid("parent is not a regular directory"));
        }
        self.read_epoch_studio_plain(&self.studio_epoch_path(scope))
    }
    pub(super) fn read_epoch_studio_plain(
        &self,
        path: &Path,
    ) -> Result<Option<AuthenticatedEpochFileBytes>, AppError> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(value) => value,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(AppError::Io(e.to_string())),
        };
        if !regular_file(&metadata) || metadata.len() > MAX_SEALED_BYTES as u64 {
            return Err(invalid("file is not bounded and regular"));
        }
        let file = File::open(path).map_err(|e| AppError::Io(e.to_string()))?;
        if !regular_file(&file.metadata().map_err(|e| AppError::Io(e.to_string()))?) {
            return Err(invalid("opened file not regular"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_SEALED_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| AppError::Io(e.to_string()))?;
        if bytes.len() > MAX_SEALED_BYTES {
            return Err(invalid("file exceeds bound"));
        }
        Ok(Some(AuthenticatedEpochFileBytes {
            plain: Zeroizing::new(unseal(&self.keys.db_key()?, &unframe(&bytes)?)?),
            physical_bytes: bytes.len() as u64,
        }))
    }
}
fn current_member(group: &ServerGroup, device: &MlsDevice) -> Result<(), AppError> {
    if group.member_signature_key(&device.device_id()).as_deref()
        != Some(device.public_key_bytes().as_slice())
    {
        return Err(invalid("local device is not a current member"));
    }
    Ok(())
}
fn decode_record<'a>(
    bytes: &'a [u8],
    scope: &[u8],
    logical: &LogicalDocument,
) -> Result<(StudioTarget, &'a [u8]), AppError> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(invalid("record exceeds bound"));
    }
    let mut d = Decoder::new(bytes);
    if d.get_bytes().map_err(invalid)? != scope {
        return Err(invalid("wrong sealed scope"));
    }
    let channel = d
        .get_bytes()
        .map_err(invalid)?
        .try_into()
        .map_err(|_| invalid("bad channel"))?;
    let target = match logical.doc_type {
        catcoms_wire::DocType::StudioIndex => StudioTarget::Index { channel },
        catcoms_wire::DocType::StudioObject => StudioTarget::Flipnote {
            channel,
            object: logical
                .logical_key
                .as_slice()
                .try_into()
                .map_err(|_| invalid("bad object"))?,
        },
        _ => return Err(invalid("wrong type")),
    };
    if target.document(&logical.server_id).map_err(invalid)? != *logical {
        return Err(invalid("wrong target"));
    }
    let snapshot = d.get_bytes().map_err(invalid)?;
    d.finish().map_err(invalid)?;
    Ok((target, snapshot))
}
pub(super) fn scope_bytes(server: u64, document: &LogicalDocument) -> Result<Vec<u8>, AppError> {
    if document.server_id.is_empty()
        || document.server_id.len() > 256
        || document.logical_key.len() != 16
        || !matches!(
            document.doc_type,
            catcoms_wire::DocType::StudioIndex | catcoms_wire::DocType::StudioObject
        )
    {
        return Err(invalid("invalid Studio scope"));
    }
    let mut e = Encoder::new();
    e.put_bytes(RECORD_DOMAIN).expect("constant fits");
    e.put_u64(server);
    e.put_bytes(&document.server_id).map_err(invalid)?;
    e.put_u16(document.doc_type.tag());
    e.put_bytes(&document.logical_key).map_err(invalid)?;
    Ok(e.finish())
}
pub(super) fn inventory_record(
    bytes: &[u8],
    server: u64,
    document: &LogicalDocument,
    scope: &[u8],
    size: u64,
) -> Result<StorageRecord, AppError> {
    let (target, snapshot) = decode_record(bytes, scope, document)?;
    let protocol = StudioEpoch::validate_vault_snapshot(snapshot, &document.server_id, target)
        .map_err(invalid)?;
    storage_record(server, document, scope, size, protocol)
}
pub(super) fn inventory_references(
    bytes: &[u8],
    server: u64,
    document: &LogicalDocument,
    scope: &[u8],
    size: u64,
) -> Result<(StorageRecord, std::collections::BTreeSet<[u8; 32]>), AppError> {
    let (target, snapshot) = decode_record(bytes, scope, document)?;
    let (protocol, cids) =
        StudioEpoch::inspect_vault_references(snapshot, &document.server_id, target)
            .map_err(invalid)?;
    Ok((
        storage_record(server, document, scope, size, protocol)?,
        cids,
    ))
}
fn storage_record(
    server: u64,
    document: &LogicalDocument,
    scope: &[u8],
    bytes: u64,
    protocol: usize,
) -> Result<StorageRecord, AppError> {
    let protocol = protocol as u64;
    Ok(StorageRecord {
        id: *blake3::hash(scope).as_bytes(),
        document: *blake3::hash(&super::epoch_recovery::scope_bytes(server, document)?).as_bytes(),
        footprint: Footprint {
            content: bytes
                .checked_sub(protocol)
                .ok_or_else(|| invalid("invalid protocol size"))?,
            protocol,
            settlement: 0,
        },
    })
}
fn sync_studio(path: &Path, expected: u64) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path).map_err(|e| AppError::Io(e.to_string()))?;
    if !regular_file(&metadata) || metadata.len() != expected {
        return Err(invalid("retry file changed"));
    }
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| AppError::Io(e.to_string()))?;
    let metadata = file.metadata().map_err(|e| AppError::Io(e.to_string()))?;
    if !regular_file(&metadata) || metadata.len() != expected {
        return Err(invalid("opened retry file changed"));
    }
    file.sync_all().map_err(|e| AppError::Io(e.to_string()))?;
    sync_directory(path.parent().ok_or_else(|| invalid("missing parent"))?)
        .map_err(|e| AppError::Io(e.to_string()))
}
fn invalid(error: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("epoch studio: {error}"))
}

#[cfg(test)]
mod tests;
