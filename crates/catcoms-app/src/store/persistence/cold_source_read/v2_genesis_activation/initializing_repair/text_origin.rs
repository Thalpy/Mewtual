//! Filled repaired pin custody only. Never select a target or repair/initialize an identity.
use super::*;

impl SavedColdV2GenesisRepairedHead {
    /// The repaired owner's SAME completed original candidate supplies stable context.
    /// A missing OnceLock refuses; this loan cannot select/reseal/repair an earlier head.
    pub(in crate::store) fn text_genesis_context_data(
        &self,
    ) -> Result<(&SavedSourceExpectationData, &[u8]), AppError> {
        let candidates = self
            .seed
            .files
            .candidates
            .get()
            .ok_or_else(critical_namespaces::refused)?;
        Ok((self.seed.expectation_data(), &candidates.head))
    }

    pub(in crate::store) fn text_pin_data(
        &self,
    ) -> Result<(&File, catcoms_storage::kept::SessionId, &[u8]), AppError> {
        let pin = self
            .seed
            .files
            .pin
            .get()
            .ok_or_else(critical_namespaces::refused)?;
        let id = *pin
            .identity
            .get()
            .ok_or_else(critical_namespaces::refused)?;
        let candidates = self
            .seed
            .files
            .candidates
            .get()
            .ok_or_else(critical_namespaces::refused)?;
        Ok((&pin.file, id, &candidates.pin))
    }
}
