//! Bounded installed-seed read under the same exclusive store ownership as edits/settlement.
//! This never generates a seed, creates a missing bucket, or advances owner finality.
use super::*;

impl ServerStore {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn read_registry_seed_prepared(
        &self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        id: u128,
        hash: [u8; 32],
        prepared: Option<(
            &super::RegistrySourceStamp,
            &mut catcoms_replication::registry_epoch::catchup::RegistryPageSource,
        )>,
        budget: &mut EpochStorageBudget,
    ) -> Result<Option<Vec<u8>>, AppError> {
        self.checked_registry_checkpoint_source(
            server,
            group,
            bucket,
            device,
            prepared.as_ref().map(|(stamp, source)| (*stamp, &**source)),
            budget,
        )?;
        prepared
            .map(|(_, source)| source.checkpoint_bytes_by_hash(id, hash).map_err(invalid))
            .transpose()
            .map(Option::flatten)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn read_registry_seed(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        doc_id: u128,
        hash: [u8; 32],
        budget: &mut EpochStorageBudget,
    ) -> Result<Option<Vec<u8>>, AppError> {
        if group.member_signature_key(&device.device_id()).as_deref()
            != Some(device.public_key_bytes().as_slice())
        {
            return Err(invalid("seed provider is not a current member"));
        }
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        // Check even absence against the complete inventory. A lost or corrupt indexed file
        // is not a valid 'unavailable' answer, and must stop further budget admission.
        let loaded = (|| {
            let bytes = self.read_registry_record(&scope)?;
            let unit = bytes
                .as_ref()
                .map(|bytes| {
                    let (stored_bucket, snapshot) = decode_record(&bytes.plain, &scope, &document)?;
                    if stored_bucket != bucket {
                        return Err(invalid("wrong bucket"));
                    }
                    RegistryEpoch::restore(snapshot, group, bucket, device.device_id())
                        .map_err(invalid)
                })
                .transpose()?;
            let record = bytes
                .as_ref()
                .zip(unit.as_ref())
                .map(|(bytes, unit)| {
                    storage_record(
                        server,
                        &document,
                        &scope,
                        bytes.physical_bytes,
                        unit.storage_protocol_bytes().map_err(invalid)?,
                    )
                })
                .transpose()?;
            Ok::<_, AppError>((unit, record))
        })();
        let (unit, record) = match loaded {
            Ok(value) => value,
            Err(error) => {
                budget.invalidate();
                return Err(error);
            }
        };
        budget
            .verify_record(&storage_scope, *blake3::hash(&scope).as_bytes(), record)
            .map_err(invalid)?;
        unit.map(|mut unit| unit.checkpoint_bytes_by_hash(doc_id, hash).map_err(invalid))
            .transpose()
            .map(Option::flatten)
    }
}
