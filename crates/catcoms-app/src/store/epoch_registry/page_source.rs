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
        Ok(blake3::hash(&record.plain) == stamp.digest)
    }
}
