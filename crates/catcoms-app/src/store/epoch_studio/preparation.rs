//! Detached verification into the existing sole warm slot. Captures contain authenticated
//! plaintext and public context only, never a store handle, vault key or MLS/device secret.
use super::*;
use catcoms_crypto::DeviceId;

pub(crate) struct StudioSourceCapture {
    mount: Arc<()>,
    server: u64,
    group: Vec<u8>,
    target: StudioTarget,
    actor: DeviceId,
    owner: DeviceId,
    mls: u64,
    plain: Zeroizing<Vec<u8>>,
    physical: u64,
}
pub(crate) struct PreparedStudioSource {
    state: EpochStudioState,
    actor: DeviceId,
    owner: DeviceId,
    mls: u64,
}
impl StudioSourceCapture {
    pub(crate) fn rebuild(self) -> Result<PreparedStudioSource, AppError> {
        let logical = self.target.document(&self.group).map_err(invalid)?;
        let scope = scope_bytes(self.server, &logical)?;
        let (target, snapshot) = decode_record(&self.plain, &scope, &logical)?;
        if target != self.target {
            return Err(invalid("prepared source channel changed"));
        }
        let unit = StudioEpoch::prepare_vault_source(
            snapshot,
            &self.group,
            target,
            self.actor,
            self.owner,
        )
        .map_err(invalid)?;
        let record = storage_record(
            self.server,
            &logical,
            &scope,
            self.physical,
            unit.storage_protocol_bytes().map_err(invalid)?,
        )?;
        let version = source::SourceVersion::prepared(
            self.mount,
            self.server,
            blake3::hash(&self.plain),
            record,
            self.physical,
        );
        Ok(PreparedStudioSource {
            state: EpochStudioState {
                unit,
                source: Some(version),
            },
            actor: self.actor,
            owner: self.owner,
            mls: self.mls,
        })
    }
}
fn owner(group: &ServerGroup) -> Result<DeviceId, AppError> {
    group
        .designated_committer()
        .ok_or_else(|| invalid("no current owner"))
}
impl ServerStore {
    /// A caller reserves the shared process-wide preparation slot BEFORE this bounded capture.
    /// Do not call per incoming packet: local scheduling must coalesce/back off failed work.
    pub(crate) fn capture_studio_source(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
    ) -> Result<Option<StudioSourceCapture>, AppError> {
        current_member(group, device)?;
        let logical = target.document(&group.group_id()).map_err(invalid)?;
        let scope = scope_bytes(server, &logical)?;
        let Some(record) =
            self.read_studio_record_bounded(&scope, source::MAX_RETAINED_BYTES as usize)?
        else {
            return Ok(None);
        };
        // Drop a matching stale unit before rebuilding it; never keep two mutable owners for it.
        if self.studio_source_is_warm(server, group, target, device) {
            self.studio_source = None;
        }
        Ok(Some(StudioSourceCapture {
            mount: self.registry_mount(),
            server,
            group: group.group_id(),
            target,
            actor: device.device_id(),
            owner: owner(group)?,
            mls: group.epoch(),
            plain: record.plain,
            physical: record.physical_bytes,
        }))
    }
    /// Reacquired native/runtime custody and current context are mandatory. Exact full-wrapper
    /// matching catches edits, faults and deletion during detached work without replaying it.
    pub(crate) fn install_prepared_studio_source(
        &mut self,
        group: &ServerGroup,
        device: &MlsDevice,
        prepared: PreparedStudioSource,
    ) -> Result<bool, AppError> {
        current_member(group, device)?;
        if prepared.actor != device.device_id()
            || prepared.owner != owner(group)?
            || prepared.mls != group.epoch()
            || prepared.state.unit.document().server_id != group.group_id()
            || !self.studio_source_bytes_match(&prepared.state)?
        {
            return Ok(false);
        }
        self.retain_studio_source(group, device, prepared.state);
        Ok(true)
    }
}
