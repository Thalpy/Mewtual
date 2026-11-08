//! Fixed-error load used only after the DISTINCT text-current policy is installed.
//! No legacy fallback or mutative SDK admission is introduced here.
use super::*;
impl ServerGroup {
    pub(crate) fn load_text_current(device: &MlsDevice, group_id: &[u8]) -> Result<Self, MlsError> {
        let gid = GroupId::from_slice(group_id);
        let group = MlsGroup::load(device.provider().storage(), &gid)
            .map_err(|_| MlsError::Internal("closed text-current SDK group restore refused"))?
            .ok_or(MlsError::Internal(
                "closed text-current SDK group is missing",
            ))?;
        Ok(Self {
            group,
            catalog: Default::default(),
            sdk_graph_lease: None,
        })
    }
}
