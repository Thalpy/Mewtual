//! The detached source half of a Registry repair job (Agent 3 design 10.3). S1 captures the
//! bucket's authenticated plaintext under custody, S2 rebuilds it with no store, vault key or MLS
//! state, and S3 hands the rebuild to the unchanged repair transaction, which uses it only if
//! every captured coordinate still matches the live group and the bytes now on disk. Studio's
//! equivalent is `StudioSourceCapture` with `install_prepared_studio_source`.
use super::*;
use catcoms_crypto::DeviceId;
use catcoms_replication::ReceiptRepair;
use std::sync::Arc;

/// Bounded authenticated plaintext and the public context it was read under. Holds no store,
/// vault key or device secret, so it may cross into a detached worker.
pub(crate) struct RegistryRepairCapture {
    mount: Arc<()>,
    server: u64,
    group: Vec<u8>,
    bucket: u8,
    actor: DeviceId,
    owner: DeviceId,
    mls: u64,
    plain: Zeroizing<Vec<u8>>,
    physical: u64,
}

/// A rebuilt bucket bound to the exact capture it came from. Not authority to write: the
/// transaction that receives it rechecks the context and the on-disk bytes first.
pub(crate) struct PreparedRegistryRepair {
    pub(super) unit: RegistryEpoch,
    mount: Arc<()>,
    server: u64,
    group: Vec<u8>,
    bucket: u8,
    actor: DeviceId,
    owner: DeviceId,
    mls: u64,
    digest: blake3::Hash,
    physical: u64,
}

impl RegistryRepairCapture {
    /// S2. CPU-heavy verification only, on a worker that owns nothing else.
    pub(crate) fn rebuild(self) -> Result<PreparedRegistryRepair, AppError> {
        let logical = registry_document(&self.group, self.bucket).map_err(invalid)?;
        let scope = scope_bytes(self.server, &logical)?;
        let (bucket, snapshot) = decode_record(&self.plain, &scope, &logical)?;
        if bucket != self.bucket {
            return Err(invalid("wrong bucket"));
        }
        let unit = RegistryEpoch::prepare_vault_source(
            snapshot,
            &self.group,
            self.bucket,
            self.actor,
            self.owner,
        )
        .map_err(invalid)?;
        Ok(PreparedRegistryRepair {
            unit,
            digest: blake3::hash(&self.plain),
            mount: self.mount,
            server: self.server,
            group: self.group,
            bucket: self.bucket,
            actor: self.actor,
            owner: self.owner,
            mls: self.mls,
            physical: self.physical,
        })
    }
}

impl PreparedRegistryRepair {
    /// Flow D evidence from this rebuild: the same assembly as `registry_repair_evidence`,
    /// without a second restore under custody.
    pub(crate) fn offered_evidence(
        &self,
        repair: &ReceiptRepair,
        offered: Option<&Receipt>,
    ) -> OfferedRepairEvidence {
        super::repair::offered_evidence_in(&self.unit, repair, offered)
    }

    /// The live context still matches the capture: same mount, server, group, bucket, actor,
    /// designated owner and MLS epoch. The on-disk bytes are checked separately.
    pub(super) fn context_matches(
        &self,
        store: &ServerStore,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
    ) -> bool {
        Arc::ptr_eq(&self.mount, &store.registry_mount())
            && self.server == server
            && self.group == group.group_id()
            && self.bucket == bucket
            && self.actor == device.device_id()
            && Some(self.owner) == group.designated_committer()
            && self.mls == group.epoch()
    }

    /// The bytes just re-read under custody are exactly the ones this was rebuilt from.
    pub(super) fn record_matches(&self, record: &AuthenticatedEpochFileBytes) -> bool {
        blake3::hash(&record.plain) == self.digest && record.physical_bytes == self.physical
    }
}

impl ServerStore {
    /// S1. The caller reserves its preparation slot BEFORE this bounded read. `None` is actual
    /// absence: a repair never creates a source.
    pub(crate) fn capture_registry_repair_source(
        &self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
    ) -> Result<Option<RegistryRepairCapture>, AppError> {
        if group.member_signature_key(&device.device_id()).as_deref()
            != Some(device.public_key_bytes().as_slice())
        {
            return Err(invalid("receiver is not a current member"));
        }
        let owner = group
            .designated_committer()
            .ok_or_else(|| invalid("no current owner"))?;
        let logical = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &logical)?;
        let Some(record) = self.read_registry_record(&scope)? else {
            return Ok(None);
        };
        Ok(Some(RegistryRepairCapture {
            mount: self.registry_mount(),
            server,
            group: group.group_id(),
            bucket,
            actor: device.device_id(),
            owner,
            mls: group.epoch(),
            plain: Zeroizing::new(record.plain.to_vec()),
            physical: record.physical_bytes,
        }))
    }

    /// S3, before the commit builds its budget: if the rebuild is still current, memoize its
    /// bucket's inventory footprint, as `cache_registry_source_footprint` does for a prepared
    /// page source. The budget's inventory scan then finds this record warm instead of validating
    /// it inline, which the receive scan refuses for cold records over its cold-byte limit. That
    /// threw away a rebuild S2 had already validated (PR #27 review MEDIUM-1; installing a Studio
    /// rebuild first is the Studio counterpart). The entry can only ever serve these exact bytes:
    /// the cache answers an exact (record, physical size, plaintext digest) match, and the scan
    /// still authenticates the whole wrapper on a hit. Returns whether the rebuild is current;
    /// `false` is a stale rebuild, and nothing is memoized.
    pub(crate) fn warm_registry_repair_inventory(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        prepared: &PreparedRegistryRepair,
    ) -> Result<bool, AppError> {
        if !self.registry_repair_source_is_current(server, group, bucket, device, prepared)? {
            return Ok(false);
        }
        let logical = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &logical)?;
        let record = storage_record(
            server,
            &logical,
            &scope,
            prepared.physical,
            prepared.unit.storage_protocol_bytes().map_err(invalid)?,
        )?;
        self.inventory_cache.put(
            (crate::store::EpochRecordKind::Registry, record.id),
            prepared.physical,
            prepared.digest,
            record,
        );
        Ok(true)
    }

    /// Whether a rebuild can still be used: the live context matches and the bucket on disk is
    /// byte-for-byte what was captured. A `false` here is a stale rebuild, not a fault.
    pub(crate) fn registry_repair_source_is_current(
        &self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        prepared: &PreparedRegistryRepair,
    ) -> Result<bool, AppError> {
        if !prepared.context_matches(self, server, group, bucket, device) {
            return Ok(false);
        }
        let logical = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &logical)?;
        Ok(self
            .read_registry_record(&scope)?
            .is_some_and(|record| prepared.record_matches(&record)))
    }
}
