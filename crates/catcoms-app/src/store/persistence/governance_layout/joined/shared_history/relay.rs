//! Ordinary indexed relay reads the actual mounted selector, never a full
//! catalogue. Only local maintenance may install/settle the archive owner.
use super::*;
use crate::store::persistence::v2_shared_history::{
    disk::indexed::INDEXED_ACTION_PAGE_WORK_FEE, SelectedSharedHistoryRelayPage,
    SharedHistoryRelayPageError,
};

impl ServerStore {
    /// Local maintenance only. This keeps the old recovery policy and returns
    /// the constructor-selected mode; no file presence or remote request can
    /// upgrade an ID8 mount. Ordinary indexed paging needs no relay catalogue.
    pub(in crate::store) fn prepare_shared_history_relay(
        &self,
        scope: SharedHistoryScope,
    ) -> Result<bool, AppError> {
        let indexed = {
            let slot = self.shared_history_mount.try_lock().map_err(|_| denied())?;
            let mounted = slot.as_ref().ok_or_else(denied)?;
            mounted.require(self)?;
            mounted.indexed.is_some()
        };
        if indexed {
            self.with_shared_history(scope, |_, _, _, _, _| Ok(()))?;
        }
        Ok(indexed)
    }

    /// One independent work debit scans at most eight raw rows. Supplying the
    /// whole prior paid page preserves its private exact-root cursor. No WAL
    /// recovery, body retention, or signing occurs on this remote read path.
    pub(in crate::store) fn selected_shared_history_relay_page(
        &self,
        scope: SharedHistoryScope,
        previous: Option<&SelectedSharedHistoryRelayPage>,
    ) -> Result<SelectedSharedHistoryRelayPage, SharedHistoryRelayPageError> {
        let cursor = match previous {
            Some(page) => Some(
                page.next_cursor()
                    .ok_or(SharedHistoryRelayPageError::Stale)?,
            ),
            None => None,
        };
        self.with_selected_shared_history(scope, None, |disk, _, key, pool| {
            let work = pool.try_reserve_classified(INDEXED_ACTION_PAGE_WORK_FEE, || {
                AppError::ChatV2ArchiveBusy
            })?;
            // Nest the typed result so the outer mounted-file final check
            // still runs, including when the inner cursor reports Stale.
            Ok(disk.indexed_action_page(key, cursor, None, &self._session, work))
        })?
    }

    /// Final negative on the SAME physical mount and retained Head. It grants
    /// no source or author permission and performs no adoption or repair.
    pub(in crate::store) fn require_selected_shared_history_relay_page(
        &self,
        scope: SharedHistoryScope,
        page: &SelectedSharedHistoryRelayPage,
    ) -> Result<(), SharedHistoryRelayPageError> {
        self.with_selected_shared_history(scope, None, |disk, _, key, _| {
            Ok(disk.require_indexed_relay_page(key, page, &self._session))
        })?
    }
}
