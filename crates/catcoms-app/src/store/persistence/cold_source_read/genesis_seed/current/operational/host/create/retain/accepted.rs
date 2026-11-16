//! Owning accepted-history lookup, distinct from the original token/retention lookup.
//! The actual query object and its same logical reader survive every fallible return.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextAcceptedQuery, PreparedV2TextAcceptedQueryCapture,
};
use crate::store::owner_evidence::canonical::text_lane::PreparedV2TextAcceptedQueryLease;
use crate::store::persistence::v2_text::{
    PreparedV2TextAcceptedQuery, V2TextAcceptedQueryResultData,
};

pub(super) struct PendingAcceptedLookup {
    admitted: Option<AdmittedV2TextAcceptedQuery>,
    // LAST: retained path/result and reader have their independent new query payment.
    // Root's owning I handoff consumes this SAME completed query and logical reader.
    // No Source Files or active finite read may be detached through this private slot.
    pub(super) query: Option<PreparedV2TextAcceptedQuery>,
}
impl PendingAcceptedLookup {
    fn prepare(
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        cell: &PreparedV2TextRetainTransaction,
    ) -> Result<Self, AppError> {
        Ok(Self {
            admitted: None,
            query: Some(PreparedV2TextAcceptedQuery::prepare(
                store,
                owner.source(),
                cell,
            )?),
        })
    }
    pub(super) fn drain_admitted(&mut self) -> Result<(), AppError> {
        if let Some(work) = self.admitted.take() {
            self.query.as_mut().ok_or_else(refused)?.run(work)?;
        }
        Ok(())
    }
    fn retry(&mut self, store: &ServerStore, owner: &V2TextOwnerRef<'_>) -> Result<(), AppError> {
        self.drain_admitted()?;
        if !self
            .query
            .as_ref()
            .ok_or_else(refused)?
            .has_completed_data()
        {
            let query = self.query.as_mut().ok_or_else(refused)?;
            let mut reader =
                PreparedV2TextAcceptedQueryLease::prepare(store, owner.source(), query)?;
            let mut capture = Some(PreparedV2TextAcceptedQueryCapture::prepare(
                store,
                owner.source(),
                query,
            )?);
            let resources = query.resources().clone();
            let admission = store.with_v2_text_current(owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_accepted_query_into(
                    query,
                    &mut reader,
                    &mut capture,
                    &mut self.admitted,
                )
            });
            // Unadmitted or displaced metadata/Files drain outside all writers, even
            // after a late callback refusal. Actual admitted IO still finishes first.
            drop(capture.take());
            reader.drain_unused_outside_writers();
            drop(reader);
            if let Some(work) = self.admitted.take() {
                query.run(work)?;
            }
            admission?;
        }
        let query = self.query.as_ref().ok_or_else(refused)?;
        let resources = query.resources().clone();
        store.with_v2_text_current(owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            query.validate_completed_under_current_guard(&mut guard)
        })
    }
}

impl PendingRetain {
    pub(in super::super) fn retry_accepted_lookup(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
    ) -> Result<(), AppError> {
        self.drain_admitted()?;
        let cell = self.transaction.as_ref().ok_or_else(refused)?;
        if self.accepted.is_none() {
            self.accepted = Some(PendingAcceptedLookup::prepare(store, owner, cell)?);
        }
        let lookup = self.accepted.as_mut().ok_or_else(refused)?;
        lookup
            .query
            .as_ref()
            .ok_or_else(refused)?
            .require_same_retained_data(cell)?;
        lookup.retry(store, owner)
    }
    pub(in super::super) fn require_absent_accepted_data(&self) -> Result<(), AppError> {
        let query = self
            .accepted
            .as_ref()
            .ok_or_else(refused)?
            .query
            .as_ref()
            .ok_or_else(refused)?;
        query.require_same_retained_data(self.transaction.as_ref().ok_or_else(refused)?)?;
        match query.result_data()? {
            V2TextAcceptedQueryResultData::Absent => Ok(()),
            // A structural hit must resolve its canonical history; it never authorizes
            // another append or falls back to creating a second acceptance.
            V2TextAcceptedQueryResultData::Present(_) => Err(refused()),
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    pub(in crate::store) fn retry_accepted_create_lookup(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.retry_retain_create_request(layout, request, rng)?;
        let store = layout.checked_store()?;
        let owner = self.owner.borrow_current()?;
        self.request_preparation
            .as_mut()
            .ok_or_else(refused)?
            .retained
            .as_mut()
            .ok_or_else(refused)?
            .retry_accepted_lookup(store, &owner)
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn accepted_lookup_absent_for_test(&self) -> bool {
        self.request_preparation
            .as_ref()
            .and_then(|p| p.retained.as_ref())
            .is_some_and(|retained| retained.require_absent_accepted_data().is_ok())
    }
}
