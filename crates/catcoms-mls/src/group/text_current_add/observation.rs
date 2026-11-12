//! Receiver-only bridge to the existing nonserialized actual operation token.
//! There is no raw facts/bytes/graph adopter or independent restored-state path.
use super::*;

pub(in crate::group) struct MergedPrivateTextAddSource<'a> {
    cell: &'a PreparedTextCurrentAdd,
}
impl PreparedTextCurrentAdd {
    pub(super) fn observation_source(&self) -> Result<MergedPrivateTextAddSource<'_>, MlsError> {
        if self.phase != Phase::Merged
            || !self.provider_closed
            || self.outputs.is_none()
            || self.graph.group.epoch() != self.original.epoch.checked_add(1).ok_or_else(bad)?
            || self.graph.group.group.pending_commit().is_some()
        {
            return Err(bad());
        }
        Ok(MergedPrivateTextAddSource { cell: self })
    }
}
impl MergedPrivateTextAddSource<'_> {
    pub(in crate::group) fn group(&self) -> &ServerGroup {
        &self.cell.graph.group
    }
    pub(in crate::group) fn device(&self) -> &MlsDevice {
        &self.cell.graph.device
    }
    pub(in crate::group) fn original_group(&self) -> [u8; 16] {
        self.cell.original.group
    }
    pub(in crate::group) fn author(&self) -> [u8; 32] {
        self.cell.original.author
    }
    pub(in crate::group) fn subject(&self) -> [u8; 32] {
        self.cell.subject
    }
    pub(in crate::group) fn before_epoch(&self) -> u64 {
        self.cell.original.epoch
    }
    pub(in crate::group) fn reference(&self) -> [u8; 32] {
        self.cell.reference
    }
    pub(in crate::group) fn credential(&self) -> [u8; 32] {
        self.cell.credential_digest
    }
    pub(in crate::group) fn commit(&self) -> Result<&[u8], MlsError> {
        self.cell.commit.as_deref().ok_or_else(bad)
    }
}
