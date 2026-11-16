//! The original cold pin only, without re-running its strict genesis-head/seed validator.
use super::*;

impl ColdV2GenesisCurrentHead {
    /// Borrow the exact immutable ciphertext captured by THIS genuine cold genesis
    /// receipt. This neither reclassifies current O nor turns an old-head path into trust.
    pub(in crate::store) fn text_genesis_context_data(
        &self,
    ) -> Result<(&SavedSourceExpectationData, &[u8]), AppError> {
        Ok((
            self.closure.resources.expectation_data(),
            &self.closure.head.frame,
        ))
    }

    pub(in crate::store) fn text_pin_data(
        &self,
    ) -> Result<(&File, catcoms_storage::kept::SessionId, &[u8]), AppError> {
        Ok((
            &self.closure.pin.object.file,
            self.closure.pin.object.id,
            &self.closure.pin.frame,
        ))
    }
}
