//! Owning token lookup from the real original host, before any new Create producer.
//!
//! This is an authenticated selected-root query only. No None value here admits a new
//! action: complete WAL/stage classification and the separate create lane must still
//! exclude an already retained but unselected original packet before signing/export.
use super::super::owner::V2TextOwnerRef;
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextTokenQuery, PreparedV2TextTokenQueryCapture,
};
use crate::store::owner_evidence::canonical::text_lane::PreparedV2TextQueryLease;
use crate::store::persistence::v2_text::{PreparedV2TextParentCapture, PreparedV2TextTokenQuery};
use crate::store::ServerStore;
mod retain;

/// One exact token and paid selected-parent capture. Query progress survives Err without
/// inventing an empty subtree, switching parent or losing an admitted reader. Every active
/// capture is local to `retry`; its Files never become dormant host metadata.
pub(super) struct PendingTokenLookup {
    token: [u8; 32],
    query: Option<PreparedV2TextTokenQuery>,
    admitted: Option<AdmittedV2TextTokenQuery>,
    // LAST: the parent observation's small read loan outlives all original query state.
    parent: PreparedV2TextParentCapture,
}
const _: () = assert!(std::mem::size_of::<PendingTokenLookup>() <= 8192);

impl PendingTokenLookup {
    pub(super) fn prepare(
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        token: [u8; 32],
    ) -> Result<Self, AppError> {
        let parent = PreparedV2TextParentCapture::prepare(store, owner.source())?;
        Ok(Self {
            token,
            query: None,
            admitted: None,
            parent,
        })
    }

    /// Consume only the original already-admitted finite read. This must run before any
    /// new Source/Current check, including cancellation or a retry with another token.
    pub(super) fn drain_admitted(&mut self) -> Result<(), AppError> {
        if let Some(work) = self.admitted.take() {
            self.query.as_mut().ok_or_else(refused)?.run(work)?;
        }
        Ok(())
    }

    pub(super) fn retry(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        token: [u8; 32],
    ) -> Result<(), AppError> {
        self.drain_admitted()?;
        if token != self.token {
            return Err(refused());
        }
        if self.query.is_none() {
            let resources = self.parent.resources().clone();
            store.with_v2_text_current(owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                self.parent.capture(&guard).map(|_| ())
            })?;
            self.query = Some(PreparedV2TextTokenQuery::prepare(
                store,
                owner.source(),
                self.parent.selected_data().ok_or_else(refused)?,
                token,
            )?);
        }
        let query = self.query.as_mut().ok_or_else(refused)?;
        if !query.has_completed_data() {
            // Each attempt owns exactly one paid reader precursor and one whole active
            // capture. A failed run drains the real End before a NEW precursor can exist;
            // successful/lost-return completion retains its SAME reader inside Query.
            let mut reader = PreparedV2TextQueryLease::prepare(store, owner.source(), query)?;
            let mut capture = Some(PreparedV2TextTokenQueryCapture::prepare(
                store,
                owner.source(),
                query,
            )?);
            let resources = query.resources().clone();
            let admission = store.with_v2_text_current(owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_token_query_into(
                    query,
                    &mut reader,
                    &mut capture,
                    &mut self.admitted,
                )
            });
            // Drop unadmitted Files/lease and unused/displaced registry metadata OUTSIDE
            // every host writer, including a late callback failure. Admitted work retains
            // all of its own custody and must finish before that error is reported.
            drop(capture.take());
            reader.drain_unused_outside_writers();
            drop(reader);
            self.drain_admitted()?;
            admission?;
        }
        let query = self.query.as_ref().ok_or_else(refused)?;
        let resources = query.resources().clone();
        store.with_v2_text_current(owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            query.validate_completed_under_current_guard(&mut guard)
        })
    }

    /// Borrow the SAME paid query, including its actual root-reader lifetime, for the
    /// later workspace/COW caller. Parsed token DATA alone must never replace this owner.
    pub(super) fn query_data(&self) -> Option<&PreparedV2TextTokenQuery> {
        self.query
            .as_ref()
            .filter(|query| query.has_completed_data())
    }
    pub(super) fn selected_data(
        &self,
    ) -> Option<&crate::store::persistence::v2_text::SelectedV2TextHead> {
        self.parent.selected_data()
    }
}
