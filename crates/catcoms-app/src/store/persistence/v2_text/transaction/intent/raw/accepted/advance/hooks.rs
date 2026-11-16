//! Test-only own-B forwarding to its SAME closed Root custody. No SDK/model/domain
//! getter, saved receipt constructor or mutable owning-slot loan is supplied.
use super::*;
use catcoms_sync::local_chat_scope::ManagedChatScopeRevocation;
use std::sync::atomic::AtomicUsize;

type ActualHeaderState = ([u8; 32], [u8; 32], Option<[u8; 32]>, bool, bool);

impl PreparedV2TextAcceptedCreate {
    /// Stop only this original Root projection domain. Native Current remains
    /// live so admission tests distinguish its independent negative fence.
    pub(in crate::store) fn cancel_original_projection_domain_for_test(&self) {
        self.signature
            .intent
            .cancel_original_projection_domain_for_test();
    }

    pub(in crate::store) fn observe_accepted_advance_work_for_test(
        &self,
        counts: Arc<[AtomicUsize; 2]>,
        lost_returns: [bool; 2],
        cancel_domain_at: Option<usize>,
        close_scope_at: Option<(usize, Arc<dyn ManagedChatScopeRevocation>)>,
    ) -> impl Drop + 'static {
        self.signature
            .intent
            .unsigned
            .observe_accepted_advance_work_for_test(
                counts,
                lost_returns,
                cancel_domain_at,
                close_scope_at,
            )
    }

    /// Observe actual Header materializer/Core derive dispatches after B owns S.
    /// This registration cannot cancel or lend the original candidate/model/SDK.
    pub(in crate::store) fn observe_candidate_work_for_test(
        &self,
        counts: Arc<[AtomicUsize; 2]>,
    ) -> impl Drop + 'static {
        self.signature
            .intent
            .unsigned
            .observe_candidate_work_for_test(counts, false)
    }

    pub(in crate::store) fn accepted_header_state_for_test(
        &self,
    ) -> Result<ActualHeaderState, AppError> {
        self.signature
            .intent
            .unsigned
            .accepted_header_state_for_test()
    }

    /// A genuine newly selected B artifact, for damage between the actual old-math
    /// drain and final proof. Only the fixture owns/mutates this path, outside writers.
    pub(in crate::store) fn accepted_current_artifact_path_for_test(
        &self,
    ) -> Result<PathBuf, AppError> {
        let saved = self.saved.as_ref().ok_or_else(refused)?;
        let fact = saved.facts.artifacts.first().ok_or_else(refused)?;
        Ok(fact.path.clone())
    }
}
