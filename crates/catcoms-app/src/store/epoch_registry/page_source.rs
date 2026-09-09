//! Bounded capture and exact saved-version checks for detached, read-only page reconstruction.
//! Hash the authenticated wrapper, not a normalized snapshot: gate/book-only changes matter.

use super::*;
use catcoms_replication::registry_epoch::catchup::RegistryPageSource;
use std::sync::Arc;

pub(crate) struct RegistrySourceStamp {
    mount: Arc<()>,
    scope: Vec<u8>,
    logical: LogicalDocument,
    digest: blake3::Hash,
    physical_bytes: u64,
}

pub(crate) struct RegistrySourceCapture {
    stamp: RegistrySourceStamp,
    snapshot: Zeroizing<Vec<u8>>,
    bucket: u8,
}

impl RegistrySourceCapture {
    /// CPU-heavy verification only; this worker owns no store, vault key, device or MLS state.
    pub(crate) fn rebuild(self) -> Result<(RegistrySourceStamp, RegistryPageSource), AppError> {
        let source = RegistryPageSource::from_vault_snapshot(
            &self.snapshot,
            &self.stamp.logical.server_id,
            self.bucket,
        )
        .map_err(invalid)?;
        Ok((self.stamp, source))
    }
}

impl ServerStore {
    /// The Studio bootstrap just durably installed this typed Registry state. Reauthenticate
    /// and match its exact snapshot before memoizing validation; unrelated Registry writes keep
    /// their existing cold-inventory behavior. Normalized-but-different states grant no hit.
    pub(crate) fn remember_installed_registry(
        &mut self,
        server: u64,
        group: &[u8],
        bucket: u8,
        state: &mut EpochRegistryState,
    ) -> Result<(), AppError> {
        let logical = registry_document(group, bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &logical)?;
        let bytes = self
            .read_registry_record(&scope)?
            .ok_or_else(|| invalid("installed Registry missing"))?;
        let (_, snapshot) = decode_record(&bytes.plain, &scope, &logical)?;
        if snapshot != state.unit.snapshot().map_err(invalid)?.as_slice() {
            return Ok(());
        }
        let record = storage_record(
            server,
            &logical,
            &scope,
            bytes.physical_bytes,
            state.unit.storage_protocol_bytes().map_err(invalid)?,
        )?;
        self.inventory_cache.put(
            (EpochRecordKind::Registry, record.id),
            bytes.physical_bytes,
            blake3::hash(&bytes.plain),
            record,
        );
        Ok(())
    }
    /// Memoize only pure validation facts from an exactly reauthenticated prepared graph.
    /// This is the existing inventory LRU, not another graph owner or a storage allowance.
    pub(crate) fn cache_registry_source_footprint(
        &mut self,
        server: u64,
        stamp: &RegistrySourceStamp,
        source: &RegistryPageSource,
    ) -> Result<(), AppError> {
        if scope_bytes(server, &stamp.logical)? != stamp.scope
            || !self.registry_page_source_is_current(stamp)?
        {
            return Err(invalid("prepared Registry inventory source changed"));
        }
        let record = storage_record(
            server,
            &stamp.logical,
            &stamp.scope,
            stamp.physical_bytes,
            source.storage_protocol_bytes().map_err(invalid)?,
        )?;
        self.inventory_cache.put(
            (
                crate::store::EpochRecordKind::Registry,
                *blake3::hash(&stamp.scope).as_bytes(),
            ),
            stamp.physical_bytes,
            stamp.digest,
            record,
        );
        Ok(())
    }
    /// The Studio coordinator's existing local cold-work rail also applies to the optional
    /// Registry bootstrap. It may not turn a peer-selected bucket into an unbounded restore.
    pub(crate) fn registry_receive_source_fits(
        &self,
        server: u64,
        group: &[u8],
        bucket: u8,
    ) -> Result<bool, AppError> {
        let scope = scope_bytes(server, &registry_document(group, bucket).map_err(invalid)?)?;
        let parent = fs::symlink_metadata(self.dir.join("servers"))
            .map_err(|e| AppError::Io(e.to_string()))?;
        if !parent.is_dir() || is_link(&parent) {
            return Err(invalid("parent is not a regular directory"));
        }
        match fs::symlink_metadata(self.registry_epoch_path(&scope)) {
            Ok(meta) if !regular_file(&meta) => return Err(invalid("file is not regular")),
            // This is ONLY a local scheduling refusal, never a claim about valid contents.
            // No reconstruction, mutation, proof or inventory credit follows from it.
            Ok(meta) if meta.len() > 256 * 1024 => return Ok(false),
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(AppError::Io(e.to_string()))
            }
            _ => {}
        }
        self.read_registry_record_bounded(&scope, 256 * 1024)
            .map(|_| true)
    }
    /// Caller reserves a preparation/retention slot BEFORE this bounded read and allocation.
    pub(crate) fn capture_registry_page_source(
        &self,
        server: u64,
        group: &[u8],
        bucket: u8,
    ) -> Result<Option<RegistrySourceCapture>, AppError> {
        let logical = registry_document(group, bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &logical)?;
        let Some(record) = self.read_registry_record(&scope)? else {
            return Ok(None);
        };
        let (_, snapshot) = decode_record(&record.plain, &scope, &logical)?;
        Ok(Some(RegistrySourceCapture {
            stamp: RegistrySourceStamp {
                mount: self.registry_mount(),
                scope,
                logical,
                digest: blake3::hash(&record.plain),
                physical_bytes: record.physical_bytes,
            },
            snapshot: Zeroizing::new(snapshot.to_vec()),
            bucket,
        }))
    }

    /// No history replay here. Missing/replaced/corrupt records never authorize cached content.
    /// Hashing the FULL authenticated plaintext detects receipt faults even with unchanged ops.
    pub(crate) fn registry_page_source_is_current(
        &self,
        stamp: &RegistrySourceStamp,
    ) -> Result<bool, AppError> {
        if !Arc::ptr_eq(&stamp.mount, &self.registry_mount()) {
            return Ok(false);
        }
        let Some(record) = self.read_registry_record(&stamp.scope)? else {
            return Ok(false);
        };
        decode_record(&record.plain, &stamp.scope, &stamp.logical)?;
        Ok(blake3::hash(&record.plain) == stamp.digest
            && record.physical_bytes == stamp.physical_bytes)
    }

    /// Reuse the read-only graph's already-verified facts, but reauthenticate its exact wrapper
    /// and physical inventory first. A missing preparation may report ONLY checked true absence.
    pub(super) fn checked_registry_checkpoint_source(
        &self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        prepared: Option<(&RegistrySourceStamp, &RegistryPageSource)>,
        budget: &mut EpochStorageBudget,
    ) -> Result<(Option<Receipt>, Option<StorageRecord>), AppError> {
        let result = (|| {
            if group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
            {
                return Err(invalid("source provider is not a member"));
            }
            let logical = registry_document(&group.group_id(), bucket).map_err(invalid)?;
            let scope = scope_bytes(server, &logical)?;
            let (head, record) = if let Some((stamp, source)) = prepared {
                if stamp.logical != logical
                    || stamp.scope != scope
                    || !self.registry_page_source_is_current(stamp)?
                {
                    return Err(invalid("prepared registry source changed"));
                }
                (
                    source.receipt_head().map_err(invalid)?.cloned(),
                    Some(storage_record(
                        server,
                        &logical,
                        &scope,
                        stamp.physical_bytes,
                        source.storage_protocol_bytes().map_err(invalid)?,
                    )?),
                )
            } else {
                self.read_registry_record_bounded(&scope, 0)?;
                (None, None)
            };
            budget
                .verify_record(
                    &StorageScope::new(server, &logical.server_id).map_err(invalid)?,
                    *blake3::hash(&scope).as_bytes(),
                    record,
                )
                .map_err(invalid)?;
            Ok((head, record))
        })();
        result.inspect_err(|_| budget.invalidate())
    }
}
