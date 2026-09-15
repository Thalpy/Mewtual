//! A page crosses one atomic vault barrier. Validation operates on a detached restored epoch;
//! a bad middle operation drops the entire unsaved page rather than committing a valid prefix.
use super::*;
use catcoms_replication::registry_epoch::catchup::{
    MAX_REGISTRY_PAGE_BYTES, MAX_REGISTRY_PAGE_OPS,
};

/// Durable admission counts, never network delivery, owner finality, or proof of currency.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegistryPageAdmission {
    pub accepted: usize,
    pub duplicates: usize,
}

impl ServerStore {
    /// Save one dependency-ordered page through the existing typed gate and accounted
    /// atomic replacement. A stale concrete epoch or a sealed gate refuses without quarantining
    /// half a page. Exact duplicates still sync the existing file and parent before returning.
    /// After an uncertain write, reconcile the budget and retry the SAME page: all-or-none
    /// replacement and per-operation dedup make both possible outcomes safe.
    #[allow(clippy::too_many_arguments)]
    pub fn ingest_registry_page(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        expected_doc_id: u128,
        device: &MlsDevice,
        operations: &[SealedOp],
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<(RegistryPageAdmission, Option<EpochRegistryState>), AppError> {
        self.ingest_registry_page_with_io(
            server,
            group,
            bucket,
            expected_doc_id,
            device,
            operations,
            rng,
            budget,
            atomic_write,
            sync_registry,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn ingest_registry_page_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        expected_doc_id: u128,
        device: &MlsDevice,
        operations: &[SealedOp],
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<(RegistryPageAdmission, Option<EpochRegistryState>), AppError> {
        if operations.len() > MAX_REGISTRY_PAGE_OPS {
            return Err(invalid("registry page operation count is invalid"));
        }
        if group.member_signature_key(&device.device_id()).as_deref()
            != Some(device.public_key_bytes().as_slice())
        {
            return Err(invalid("registry receiver is not a current member"));
        }
        if operations.is_empty() {
            return self
                .verify_empty_registry_page(
                    server,
                    group,
                    bucket,
                    expected_doc_id,
                    device,
                    budget,
                    sync,
                )
                .map(|state| (RegistryPageAdmission::default(), state));
        }
        let mut total = 0;
        for op in operations {
            if op.doc_type != catcoms_wire::DocType::DocRegistry
                || op.doc_id != expected_doc_id
                || op.epoch != group.epoch()
                || op.blob.ciphertext.len() > MAX_INBOUND_CIPHERTEXT
            {
                return Err(invalid("registry page operation scope or size is invalid"));
            }
            // SealedOp framing is exactly 58 bytes; each page entry adds a four-byte length.
            total += op.blob.ciphertext.len() + 62;
            if total > MAX_REGISTRY_PAGE_BYTES {
                return Err(invalid("registry page exceeds byte cap"));
            }
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
            |unit, _| {
                if unit.doc_id() != expected_doc_id || unit.phase() != EpochPhase::Open {
                    return Err(invalid("registry page target is replaced or not open"));
                }
                let mut result = RegistryPageAdmission::default();
                for op in operations {
                    match unit.ingest(op, group, device).map_err(invalid)? {
                        Admission::Accepted => result.accepted += 1,
                        Admission::Duplicate => result.duplicates += 1,
                        // Exclusive access and the Open precheck make this unreachable for a
                        // conforming gate. Keep it fail-closed if that contract ever changes.
                        _ => return Err(invalid("registry page was not admitted")),
                    }
                }
                Ok(result)
            },
            writer,
            sync,
        )
        .map(|(counts, state)| (counts, Some(state)))
    }

    /// A terminal-empty answer asserts no new content. Still verify inventory, exact target and
    /// Open state, and flush held bytes: an earlier uncertain rename is not a durability proof.
    /// An absent epoch-zero record stays absent; never create it to manufacture empty success.
    #[allow(clippy::too_many_arguments)]
    fn verify_empty_registry_page(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        expected_doc_id: u128,
        device: &MlsDevice,
        budget: &mut EpochStorageBudget,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<Option<EpochRegistryState>, AppError> {
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        let loaded = (|| {
            let held = self.read_registry_record(&scope)?;
            let state = held
                .as_ref()
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
                .transpose()?;
            let observed = held
                .as_ref()
                .zip(state.as_ref())
                .map(|(bytes, state)| {
                    storage_record(
                        server,
                        &document,
                        &scope,
                        bytes.physical_bytes,
                        state.unit.storage_protocol_bytes().map_err(invalid)?,
                    )
                })
                .transpose()?;
            Ok::<_, AppError>((state, observed))
        })()
        .inspect_err(|_| budget.invalidate())?;
        let (state, observed) = loaded;
        budget
            .verify_record(&storage_scope, *blake3::hash(&scope).as_bytes(), observed)
            .map_err(invalid)?;
        let doc_id = state.as_ref().map_or_else(
            || {
                catcoms_replication::epoch_zero_id(
                    catcoms_wire::DocType::DocRegistry,
                    &document.logical_key,
                )
            },
            EpochRegistryState::doc_id,
        );
        if doc_id != expected_doc_id
            || state
                .as_ref()
                .is_some_and(|state| state.phase() != EpochPhase::Open)
        {
            return Err(invalid("registry page target is replaced or not open"));
        }
        if let Some(record) = observed {
            let reservation = budget
                .reserve_sync(&storage_scope, record)
                .map_err(invalid)?;
            sync(
                &self.registry_epoch_path(&scope),
                record.footprint.total().map_err(invalid)?,
            )?;
            reservation.commit();
        }
        Ok(state)
    }
}
