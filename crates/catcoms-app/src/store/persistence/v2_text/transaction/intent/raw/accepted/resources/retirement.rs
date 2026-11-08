//! Exactly one real retirement-root preparation for the SAME B metadata account.
//! The private Factory Arc has only its bounded prepared/closed/drained/removed
//! aliases; generation arrays/refs must drain BEFORE this final claim/payment tail.
use super::*;
pub(in crate::store) struct V2TextRetirementRootsClaim {
    metadata: AcceptedMetadataResources,
}
impl AcceptedMetadataResources {
    pub(in crate::store) fn claim_retirement_roots_preparation(
        &self,
    ) -> Result<V2TextRetirementRootsClaim, AppError> {
        self.validate_prepaid_account()?;
        self.body
            .retirement_roots_busy
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .map_err(|_| refused())?;
        Ok(V2TextRetirementRootsClaim {
            metadata: self.clone(),
        })
    }
}
impl Drop for V2TextRetirementRootsClaim {
    fn drop(&mut self) {
        self.metadata
            .body
            .retirement_roots_busy
            .store(false, std::sync::atomic::Ordering::Release);
    }
}
