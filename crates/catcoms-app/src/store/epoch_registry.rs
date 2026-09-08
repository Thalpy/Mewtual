//! Durable registry epochs. Reload, gated mutation, accounting and atomic vault save
//! share one exclusive store borrow. No caller can replace a saved epoch with an arbitrary view.
//! Local edits save intents before changes. Installation saves recovery and covered-intent
//! retirement before selecting a seed-backed successor; live replay/discovery remain separate.
//! Returned ciphertext is prepared for, not proof of, network publication.

use std::io::Read;

use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_replication::epoch::{MAX_RECEIPT_BYTES, MAX_SIGNED_EPOCH_OP_BYTES};
use catcoms_replication::registry::{registry_document, RegistryOp, RegistryProjection};
use catcoms_replication::registry_epoch::{
    RegistryEpoch, RegistrySettlementPlan, MAX_REGISTRY_EPOCH_SNAPSHOT_BYTES,
};
use catcoms_replication::{
    Admission, DomainOp, EpochPhase, LogicalDocument, Receipt, ReceiptIngest, SealedOp,
};

use super::epoch_budget::{
    EpochStorageBudget, Footprint, Replacement, StorageRecord, StorageScope, WritePurpose,
};
use super::epoch_recovery::inventory::{is_link, regular_file};
use super::epoch_recovery::AuthenticatedEpochFileBytes;
use super::*;

pub(super) const RECORD_DOMAIN: &[u8] = b"catcoms/epoch-registry-store/v1";
const MAX_RECORD_BYTES: usize = MAX_REGISTRY_EPOCH_SNAPSHOT_BYTES + 1024;
pub(super) const MAX_SEALED_BYTES: usize = MAX_RECORD_BYTES + 40;
// Every accepted inner op fits a 256-KiB padding bucket, plus the authenticated length footer
// and AEAD tag. Bound the public struct before SealedOp::open can allocate plaintext.
const MAX_INBOUND_CIPHERTEXT: usize = MAX_SIGNED_EPOCH_OP_BYTES + 4 + 16;

mod head;
mod installation;
mod pass;
mod receive;
pub use receive::RegistryPageAdmission;
mod recovery;
mod replay;
pub use installation::RegistryInstallOutcome;
pub use pass::{
    RegistryReplayPass, RegistryReplayProgress, RegistryReplayStep, RegistryReplayTicket,
};
pub use replay::{RegistryReplayHold, RegistryReplayOutcome};

/// Detached read-only persisted state. Debug deliberately excludes registry keys and content.
pub struct EpochRegistryState {
    unit: RegistryEpoch,
}

impl std::fmt::Debug for EpochRegistryState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochRegistryState")
            .field("epoch", &self.epoch())
            .field("phase", &self.phase())
            .field("operations", &self.op_count())
            .finish_non_exhaustive()
    }
}

impl EpochRegistryState {
    /// Starting claims computed only from the checked saved unit, not supplied by the renderer.
    pub(crate) fn catchup_frontier(
        &mut self,
    ) -> catcoms_replication::registry_epoch::catchup::RegistryFrontier {
        self.unit.catchup_frontier()
    }
    /// Read-only page from the already authenticated/rebuilt unit. The Server adapter checks
    /// runtime/mount binding and request authority before loading this potentially large file.
    pub(crate) fn catchup_page(
        &self,
        provider: &mut catcoms_replication::registry_epoch::catchup::RegistryPageProvider,
        group: &ServerGroup,
        device: &MlsDevice,
        request: catcoms_replication::registry_epoch::catchup::RegistryPageRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<catcoms_replication::registry_epoch::catchup::RegistryPageOutcome, AppError> {
        provider
            .page(&self.unit, group, device, request, rng)
            .map_err(invalid)
    }

    /// Capture this id when preparing an edit; retries must keep it across rotations.
    pub fn doc_id(&self) -> u128 {
        self.unit.doc_id()
    }

    /// Concrete retained epoch, not evidence that settlement/pruning has completed.
    pub fn epoch(&self) -> u64 {
        self.unit.epoch()
    }
    /// Admission phase which was durably saved, except that a loaded state alone grants no ack.
    pub fn phase(&self) -> EpochPhase {
        self.unit.phase()
    }
    /// Full accepted source-log count; sealing never discards content.
    pub fn op_count(&self) -> usize {
        self.unit.op_count()
    }
    /// Distinct post-seal hashes retained without accepting their content.
    pub fn quarantined_len(&self) -> usize {
        self.unit.quarantined_len()
    }
    /// Detached typed projection for callers; no mutable gate/document/receipt book escapes.
    pub fn projection(&self) -> Result<RegistryProjection, AppError> {
        self.unit.projection().map_err(invalid)
    }
}

impl ServerStore {
    /// Rebuild an exact receipted registry checkpoint and its recovery inputs from the checked
    /// vault unit. Read-only: no reserve is spent, recovery saved, intent retired or log pruned.
    /// The returned plan is ephemeral, not an installation/durability permit. A future worker
    /// must reload, compare its source version and recheck authority under the per-document gate
    /// before persisting recovery and installing anything. Missing state is not an empty epoch.
    pub fn plan_registry_settlement(
        &self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        close_bytes: &[u8],
        expected_tenure_start: u64,
    ) -> Result<RegistrySettlementPlan, AppError> {
        // Reject oversized/malformed public input before reading or replaying the vault unit.
        let close = catcoms_replication::CloseRecord::decode(close_bytes).map_err(invalid)?;
        let mut state = self
            .load_registry_epoch(server, group, bucket, device)?
            .ok_or_else(|| invalid("registry settlement source is missing"))?;
        state
            .unit
            .prepare_settlement(&close, group, expected_tenure_start)
            .map_err(invalid)
    }

    /// Journal one canonical local intent, then save its checked registry edit before returning
    /// ciphertext. Reuse the SAME nonce/envelope on retry; a held edit reseals the original signed
    /// bytes, even after newer edits arrive. Both intent and epoch cross their durability barriers.
    /// A failure after the first barrier deliberately retains the intent for later retry/recovery.
    /// Keep the captured concrete `expected_doc_id` on retry too: once rotated, the old request
    /// refuses before journaling rather than reauthoring its now-retired id in a different epoch.
    /// Closing/Fault refuses both new edits and retries. No intent retirement or network send is
    /// performed; the sender must still recheck its session, group epoch and document lifecycle.
    #[allow(clippy::too_many_arguments)]
    pub fn edit_registry_epoch(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        expected_doc_id: u128,
        device: &MlsDevice,
        operation: DomainOp,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
    ) -> Result<(SealedOp, EpochRegistryState), AppError> {
        self.edit_registry_epoch_with_io(
            server,
            group,
            bucket,
            expected_doc_id,
            device,
            operation,
            rng,
            budget,
            intents,
            atomic_write,
            super::epoch_intents::sync_intent,
            atomic_write,
            sync_registry,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn edit_registry_epoch_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        expected_doc_id: u128,
        device: &MlsDevice,
        operation: DomainOp,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
        intent_writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        intent_sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
        epoch_writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        epoch_sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<(SealedOp, EpochRegistryState), AppError> {
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        // Bound and authenticate the caller before rebuilding any saved graph or copying its
        // operation into the intent ledger. RegistryOp decoding is capped at 1024 body bytes.
        if operation.doc_type != document.doc_type
            || operation.logical_key != document.logical_key
            || RegistryOp::decode(&operation.body)
                .map_err(invalid)?
                .domain_op(&document.server_id, operation.nonce)
                .map_err(invalid)?
                != operation
        {
            return Err(invalid("invalid local registry operation"));
        }
        if group.member_signature_key(&device.device_id()).as_deref()
            != Some(device.public_key_bytes().as_slice())
        {
            return Err(invalid("local registry author is not a current member"));
        }
        // Check before preparing an intent, including a conflicting id held via inbound ingest
        // with NO local ledger yet. Otherwise we could durably strand the conflicting body.
        let checked = match self.load_registry_epoch(server, group, bucket, device) {
            Ok(Some(state)) => state.unit,
            Ok(None) => RegistryEpoch::new(group, bucket, device.device_id()).map_err(invalid)?,
            Err(error) => {
                budget.invalidate();
                return Err(error);
            }
        };
        if checked.doc_id() != expected_doc_id {
            return Err(invalid("registry edit belongs to a retired epoch"));
        }
        checked
            .validate_local_edit(device, group, &operation)
            .map_err(invalid)?;
        drop(checked);
        self.prepare_epoch_intent_with_io(
            server,
            &document,
            operation.clone(),
            device,
            group,
            rng,
            budget,
            intents,
            intent_writer,
            intent_sync,
        )?;
        // The exclusive store borrow spans both records. There is no accepted edit or outbound
        // result between them. An uncertain second save never rolls back the already-safe intent.
        self.update_registry_with_io(
            server,
            group,
            bucket,
            device,
            true,
            WritePurpose::Ordinary,
            rng,
            budget,
            |unit, rng| {
                if unit.doc_id() != expected_doc_id {
                    return Err(invalid("registry edit belongs to a retired epoch"));
                }
                unit.edit_or_reseal(device, group, rng, &operation)
                    .map_err(invalid)
            },
            epoch_writer,
            epoch_sync,
        )
    }

    /// Read the checked, vault-authenticated current registry bucket, preserving historical
    /// admission across owner changes. Only absent means None; corrupt state never resets.
    pub fn load_registry_epoch(
        &self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
    ) -> Result<Option<EpochRegistryState>, AppError> {
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &document)?;
        self.read_registry_record(&scope)?
            .map(|bytes| {
                let (stored_bucket, snapshot) = decode_record(&bytes.plain, &scope, &document)?;
                if stored_bucket != bucket {
                    return Err(invalid("wrong bucket"));
                }
                Ok(EpochRegistryState {
                    unit: RegistryEpoch::restore(snapshot, group, bucket, device.device_id())
                        .map_err(invalid)?,
                })
            })
            .transpose()
    }

    /// Admit one encrypted inbound op, then persist before returning its admission outcome.
    /// A missing record can start epoch zero only; rotated IDs cannot create independent roots.
    /// Duplicate/late retries still cross a durability barrier. Caller must inspect Admission:
    /// a saved quarantine hash is not an accepted edit and must never earn an accepted-op ack.
    #[allow(clippy::too_many_arguments)]
    pub fn ingest_registry_epoch(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        sealed: &SealedOp,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<(Admission, EpochRegistryState), AppError> {
        if sealed.blob.ciphertext.len() > MAX_INBOUND_CIPHERTEXT {
            return Err(invalid("inbound ciphertext exceeds registry bound"));
        }
        self.update_registry_with_io(
            server,
            group,
            bucket,
            device,
            true,
            WritePurpose::Ordinary,
            rng,
            budget,
            |unit, _| unit.ingest(sealed, group, device).map_err(invalid),
            atomic_write,
            sync_registry,
        )
    }

    /// Persist a current-owner receipt and its gate seal together, retaining the full source.
    /// Missing source history must be fetched first. No recovery acknowledgement, source
    /// replacement or pruning is performed. Externally observed tenure evidence is mandatory.
    #[allow(clippy::too_many_arguments)]
    pub fn seal_registry_epoch(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        receipt: Receipt,
        tenure_start: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<(ReceiptIngest, EpochRegistryState), AppError> {
        // Receipt has public Vec fields; check before encoding or hashing them.
        scope_bytes(server, &receipt.document)?;
        if receipt.owner_public_key.len() != 32 || receipt.encode().len() > MAX_RECEIPT_BYTES {
            return Err(invalid("receipt exceeds its bound"));
        }
        let receipt = Receipt::decode(&receipt.encode()).map_err(invalid)?;
        if receipt.document != registry_document(&group.group_id(), bucket).map_err(invalid)? {
            return Err(invalid("receipt names another registry bucket"));
        }
        // Reject outsider/stale authority before consulting disk or accounting. A syntactically
        // valid receipt alone must not force an otherwise healthy inventory to reconcile.
        receipt
            .verify_current_owner(group, tenure_start)
            .map_err(invalid)?;
        self.update_registry_with_io(
            server,
            group,
            bucket,
            device,
            false,
            WritePurpose::Settlement,
            rng,
            budget,
            |unit, _| unit.seal(receipt, group, tenure_start).map_err(invalid),
            atomic_write,
            sync_registry,
        )
    }

    // Reload on every mutation: stale detached readers cannot overwrite newer operations. A
    // failed save returns no updated state/outcome; after uncertain rename, a retry reloads the
    // authentic final and must flush it again rather than mistake visibility for durability.
    #[allow(clippy::too_many_arguments)]
    fn update_registry_with_io<T, R: CryptoRngCore>(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        allow_create: bool,
        purpose: WritePurpose,
        rng: &mut R,
        budget: &mut EpochStorageBudget,
        apply: impl FnOnce(&mut RegistryEpoch, &mut R) -> Result<T, AppError>,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<(T, EpochRegistryState), AppError> {
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        let loaded = (|| {
            let held = self.read_registry_record(&scope)?;
            let unit = match &held {
                Some(bytes) => {
                    let (stored_bucket, snapshot) = decode_record(&bytes.plain, &scope, &document)?;
                    if stored_bucket != bucket {
                        return Err(invalid("wrong bucket"));
                    }
                    RegistryEpoch::restore(snapshot, group, bucket, device.device_id())
                        .map_err(invalid)?
                }
                None => RegistryEpoch::new(group, bucket, device.device_id()).map_err(invalid)?,
            };
            Ok((held, unit))
        })();
        let (held, mut unit) = match loaded {
            Ok(value) => value,
            Err(error) => {
                budget.invalidate();
                return Err(error);
            }
        };
        let observed = held
            .as_ref()
            .map(|bytes| {
                storage_record(
                    server,
                    &document,
                    &scope,
                    bytes.physical_bytes,
                    unit.storage_protocol_bytes().map_err(invalid)?,
                )
            })
            .transpose()?;
        budget
            .verify_record(&storage_scope, *blake3::hash(&scope).as_bytes(), observed)
            .map_err(invalid)?;
        // Expected absence is a normal fetch prerequisite, not uncertain I/O. Verify inventory
        // first so an indexed source disappearing still closes admission rather than resetting.
        if held.is_none() && !allow_create {
            return Err(invalid("registry source is missing; fetch before sealing"));
        }
        // Restore refreshes the quota-exempt owner from the current group. Compare mutation
        // against that normalized baseline, not old disk bytes, so a harmless owner refresh
        // does not require a replacement copy at the content cap. The actual old bytes still
        // cross a flush barrier, and every subsequent restore derives the owner again.
        let before = Zeroizing::new(unit.snapshot().map_err(invalid)?);
        let outcome = apply(&mut unit, rng)?;
        let snapshot = Zeroizing::new(unit.snapshot().map_err(invalid)?);
        let mut e = Encoder::new();
        e.put_bytes(&scope).map_err(invalid)?;
        e.put_u8(bucket);
        e.put_bytes(&snapshot).map_err(invalid)?;
        let plain = Zeroizing::new(e.finish());
        let path = self.registry_epoch_path(&scope);
        if held.is_some() && before.as_slice() == snapshot.as_slice() {
            let record = observed.ok_or_else(|| invalid("unchanged record is absent"))?;
            let reservation = budget
                .reserve_sync(&storage_scope, record)
                .map_err(invalid)?;
            sync(&path, record.footprint.total().map_err(invalid)?)?;
            reservation.commit();
        } else {
            let record = storage_record(
                server,
                &document,
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
        Ok((outcome, EpochRegistryState { unit }))
    }

    fn registry_epoch_path(&self, scope: &[u8]) -> PathBuf {
        self.dir
            .join("servers")
            .join(format!("{}.registry-epoch", blake3::hash(scope).to_hex()))
    }

    fn read_registry_record(
        &self,
        scope: &[u8],
    ) -> Result<Option<AuthenticatedEpochFileBytes>, AppError> {
        let metadata = fs::symlink_metadata(self.dir.join("servers"))
            .map_err(|e| AppError::Io(e.to_string()))?;
        if !metadata.is_dir() || is_link(&metadata) {
            return Err(invalid("parent is not a regular directory"));
        }
        self.read_epoch_registry_plain(&self.registry_epoch_path(scope))
    }

    pub(super) fn read_epoch_registry_plain(
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
            return Err(invalid("opened file is not regular"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_SEALED_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| AppError::Io(e.to_string()))?;
        if bytes.len() > MAX_SEALED_BYTES {
            return Err(invalid("file exceeds its bound"));
        }
        Ok(Some(AuthenticatedEpochFileBytes {
            plain: Zeroizing::new(unseal(&self.keys.db_key()?, &unframe(&bytes)?)?),
            physical_bytes: bytes.len() as u64,
        }))
    }
}

/// Parse the wrapper first, then let the shared replication validator inspect the raw seed/log.
fn decode_record<'a>(
    bytes: &'a [u8],
    scope: &[u8],
    document: &LogicalDocument,
) -> Result<(u8, &'a [u8]), AppError> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(invalid("record exceeds its bound"));
    }
    let mut d = Decoder::new(bytes);
    if d.get_bytes().map_err(invalid)? != scope {
        return Err(invalid("wrong sealed scope"));
    }
    let bucket = d.get_u8().map_err(invalid)?;
    if registry_document(&document.server_id, bucket).map_err(invalid)? != *document {
        return Err(invalid("wrong logical bucket"));
    }
    let snapshot = d.get_bytes().map_err(invalid)?;
    d.finish().map_err(invalid)?;
    Ok((bucket, snapshot))
}

pub(super) fn inventory_record(
    bytes: &[u8],
    server: u64,
    document: &LogicalDocument,
    scope: &[u8],
    size: u64,
) -> Result<StorageRecord, AppError> {
    let (bucket, snapshot) = decode_record(bytes, scope, document)?;
    let protocol = RegistryEpoch::validate_vault_snapshot(snapshot, &document.server_id, bucket)
        .map_err(invalid)?;
    storage_record(server, document, scope, size, protocol)
}

pub(super) fn scope_bytes(server: u64, document: &LogicalDocument) -> Result<Vec<u8>, AppError> {
    if document.server_id.len() > 256
        || document.logical_key.len() != 32
        || document.doc_type != catcoms_wire::DocType::DocRegistry
    {
        return Err(invalid("invalid registry scope"));
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

fn storage_record(
    server: u64,
    document: &LogicalDocument,
    scope: &[u8],
    bytes: u64,
    protocol: usize,
) -> Result<StorageRecord, AppError> {
    let protocol = protocol as u64;
    let content = bytes
        .checked_sub(protocol)
        .ok_or_else(|| invalid("invalid protocol footprint"))?;
    Ok(StorageRecord {
        id: *blake3::hash(scope).as_bytes(),
        document: *blake3::hash(&super::epoch_recovery::scope_bytes(server, document)?).as_bytes(),
        footprint: Footprint {
            content,
            protocol,
            settlement: 0,
        },
    })
}

fn sync_registry(path: &Path, expected_bytes: u64) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path).map_err(|e| AppError::Io(e.to_string()))?;
    if !regular_file(&metadata) || metadata.len() != expected_bytes {
        return Err(invalid("retry file changed"));
    }
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| AppError::Io(e.to_string()))?;
    let metadata = file.metadata().map_err(|e| AppError::Io(e.to_string()))?;
    if !regular_file(&metadata) || metadata.len() != expected_bytes {
        return Err(invalid("opened retry file changed"));
    }
    file.sync_all().map_err(|e| AppError::Io(e.to_string()))?;
    sync_directory(path.parent().ok_or_else(|| invalid("missing parent"))?)
        .map_err(|e| AppError::Io(e.to_string()))
}

fn invalid(error: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("epoch registry: {error}"))
}

#[cfg(test)]
mod tests;
