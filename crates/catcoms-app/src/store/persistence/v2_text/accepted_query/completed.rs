//! Original-bound completed Directory DATA for the paired B COW producer.
//! This does not renew its logical reader or confer Current/miss/retirement authority.
use super::*;

impl PreparedV2TextAcceptedQuery {
    pub(in crate::store) fn borrow_completed_directory_path_data(
        &self,
        retained: &PreparedV2TextRetainTransaction,
    ) -> Result<&AcceptedIndexPathData, AppError> {
        self.result_data()?;
        self.plan
            .parent
            .original
            .require_same_original(&retained.observe_original())?;
        self.plan
            .parent
            .resources
            .require_same_original(retained.resources())?;
        self.completed_reader
            .as_ref()
            .ok_or_else(refused)?
            .require_same_query(&self.observe_original())?;
        let directory = self.directory.as_ref().ok_or_else(refused)?;
        if directory.next_page_data()?.is_some() {
            return Err(refused());
        }
        Ok(directory)
    }
}
