//! Persistent original JOIN source/debt is negative exclusion even without a runtime task.
//! A cold mount must not let an old generic admission overwrite its exact original basis.
use super::*;

impl ServerStore {
    /// NEGATIVE check only. Caller MUST already hold the actual admission serializer, before
    /// taking pins/head/Root writers or starting target/canonical I/O. This takes only the
    /// subordinate fixed-five catalog owner and releases it on return. All actual JOIN writes
    /// admit their candidate under the SAME admission->catalog order, so no matching source
    /// can appear during the caller's later synchronous effect while admission remains held.
    ///
    /// Weak task expiry, Paused scheduling, cold reopen and an unrenamed Store-owned candidate
    /// do not mean source/debt absence. This grants no permission to initialize/resume/retire,
    /// and must never be used as a status flag or an intent-backed positive bypass. An exact
    /// journal ScopeIntent consumer needs its own closed source-bound path when implemented.
    pub(in crate::store) fn require_no_pending_checkpoint_join_source(
        &self,
        group: &[u8],
        signer: &[u8; 32],
    ) -> Result<(), AppError> {
        if self.pending_join_policy.is_none() {
            return Ok(());
        }
        let mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
        let mounted = mounted.as_ref().ok_or_else(refused)?;
        let matches = |summary: &codec::Summary| {
            summary
                .rows
                .iter()
                .flatten()
                .any(|row| row.group.as_slice() == group && &row.signer == signer)
        };
        if matches(&mounted.current)
            || mounted
                .pending
                .as_ref()
                .is_some_and(|candidate| matches(&candidate.expected) || matches(&candidate.next))
        {
            return Err(refused());
        }
        Ok(())
    }
}
