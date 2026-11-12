//! Cached original reader/source-slot tail for a caller that has ALREADY completed
//! its final physical source proof while holding SAME canonical Current. No file,
//! account, codec, allocation, new lease or miss authority is created here.
use super::*;
impl PreparedV2TextTokenQuery {
    pub(in crate::store::persistence::v2_text) fn require_completed_reader_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        let reader = self.completed_reader.as_ref().ok_or_else(refused)?;
        reader.require_same_query(&self.observe_original())?;
        reader.require_live_under_current_guard(guard, &self.observe_original())?;
        reader.require_live_query(&self.observe_original())?;
        guard.validate_source_only()
    }
}
