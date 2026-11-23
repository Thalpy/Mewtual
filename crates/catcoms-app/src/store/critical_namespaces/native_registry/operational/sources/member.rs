//! Existing-runtime member operations. The original Sources stays canonical;
//! finite work drops all book/Entry issuer pins before its runtime writer/IO.
use super::*;
use crate::store::{V2MemberSaveOutcome, V2MemberSaveRefusal, V2MemberSignRequest};
use catcoms_rt::{NativeChatPreparedKeyCut, RequestCancellation};
mod history;
pub use history::{
    NativeTextMemberHistoryItem, NativeTextMemberHistoryPage, NativeTextMemberHistoryStatus,
};
use std::sync::Weak;
mod admission;
mod connected;
mod publication;
mod receiving;
pub use admission::{
    NativeTextMemberAdmissionFailure, NativeTextMemberFinalPreparation, NativeTextMemberSaveJob,
};
pub use publication::{
    NativeTextFounderMemberPublication, NativeTextFounderMemberPublicationReceipt,
};
pub use receiving::{
    NativeTextFounderDependencyProvider, NativeTextFounderMemberIngressPreparation,
    NativeTextFounderMemberReceiver,
};
type MemberKeys<T, R> = (Arc<RuntimeCell<T, R>>, NativeFounderKeys);
type JoinKeys<T, R> = (MemberKeys<T, R>, [NativeChatPreparedKeyCut; 3]);

/// Independent per-invocation payment. Configuration is not a source selector
/// or native credential; the exact policy account is checked again at capture.
pub struct NativeTextMemberBudget<T, R> {
    metadata: Arc<RuntimeMetadata>,
    _types: PhantomData<fn() -> (T, R)>,
}
impl NativeTextSourcePolicy {
    /// Reserve before native Store/controller/UI guards or request copies.
    pub fn prepare_member<T: MeshTransport, R: CryptoRngCore>(
        &self,
    ) -> Result<NativeTextMemberBudget<T, R>, AppError> {
        if runtime_invoice::<T, R>().is_none_or(|n| n > ENTRY_BYTES) {
            return Err(refused());
        }
        Ok(NativeTextMemberBudget {
            metadata: Arc::new(RuntimeMetadata {
                pool: self.pool.clone(),
                _debit: self.pool.try_reserve(ENTRY_BYTES)?,
            }),
            _types: PhantomData,
        })
    }
}

/// Guard-safe owning capture, including refusal. Keep the whole capture outside
/// native guards through finish/Drop. No caller-supplied founder settings/RNG
/// can select, recreate or replace the retained original member runtime.
pub struct NativeTextMemberPreparation<T: MeshTransport, R: CryptoRngCore> {
    scope: Arc<Scope>,
    book: Weak<SourceBook<T, R>>,
    row: NativeTextRuntimeSelection<T, R>,
    command: Option<NativeTextSourceCommand>,
    work: Option<SourceWork>,
    budget: NativeTextMemberBudget<T, R>,
    refusal: Option<V2MemberSaveRefusal>,
    _end: Option<retirement::PreparationEnd>,
    // LAST even on a refused capture: its actual weak book can outlive Sources.
    _control: Arc<retirement::Control>,
}

/// Whole finite prepare work. Its concrete native member key cut is issued by
/// the original Entry, not a row label. This owns no Entry/SourceOwner/issuer.
pub struct NativeTextMemberJob<T: MeshTransport, R: CryptoRngCore> {
    pub(super) inner: NativeTextCreationJob<T, R>,
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextSources<T, R> {
    /// Atomic/Arc/Weak-only under the established short native owner borrow.
    /// Actual index IO, issuer lookup, key preparation and runtime borrowing
    /// happen only in finish, after every native/controller guard has left.
    pub fn capture_member(
        &self,
        row: &NativeTextRuntimeSelection<T, R>,
        command: NativeTextSourceCommand,
        budget: NativeTextMemberBudget<T, R>,
    ) -> NativeTextMemberPreparation<T, R> {
        let end = self.retirement.enter_preparation();
        let refusal = if !Arc::ptr_eq(&self.scope, &row.original.scope)
            || !self.policy.pool.same_pool_cached(&budget.metadata.pool)
            || self.require_command(&command).is_err()
        {
            Some(V2MemberSaveRefusal::Unavailable)
        } else if end.is_none() {
            Some(V2MemberSaveRefusal::Busy)
        } else {
            None
        };
        let work = if refusal.is_none() {
            self.work.for_paid_runtime(&budget.metadata)
        } else {
            None
        };
        NativeTextMemberPreparation {
            scope: self.scope.clone(),
            book: Arc::downgrade(&self.entries),
            row: row.clone(),
            command: Some(command),
            work,
            budget,
            refusal,
            _end: end,
            _control: self.retirement.clone(),
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextMemberPreparation<T, R> {
    fn require_current(&self) -> Result<(), V2MemberSaveRefusal> {
        if let Some(reason) = self.refusal {
            return Err(reason);
        }
        let command = self
            .command
            .as_ref()
            .ok_or(V2MemberSaveRefusal::Unavailable)?;
        if self._end.is_none()
            || !self.scope.is_open()
            || !Arc::ptr_eq(&self.scope, &command.scope)
            || !Arc::ptr_eq(&self.scope, &self.row.original.scope)
            || !command.ui.is_current_cached()
            || self.work.as_ref().is_none_or(|w| !w.is_live_cached())
        {
            return Err(V2MemberSaveRefusal::Unavailable);
        }
        Ok(())
    }

    /// Finish outside ALL native/Root guards. Fresh covered-row lookup precedes
    /// taking an Entry pin; every such pin leaves before the runtime writer.
    fn prepare_keys(&self) -> Result<MemberKeys<T, R>, V2MemberSaveRefusal> {
        self.prepare_using_issuer(NativeFounderKeys::prepare)
    }

    fn prepare_using_issuer<K>(
        &self,
        prepare: impl FnOnce(
            &NativeChatSourceOwner,
            &catcoms_rt::NativeChatUiCommand,
        ) -> Result<K, AppError>,
    ) -> Result<(Arc<RuntimeCell<T, R>>, K), V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Error;
        self.require_current()?;
        self.budget
            .metadata
            ._debit
            .validate(&self.budget.metadata.pool, ENTRY_BYTES)
            .map_err(|_| Error::Unavailable)?;
        let actual = self
            .work
            .as_ref()
            .unwrap()
            .lookup_creation(self.row.original.invocation, self.row.original.request)
            .map_err(|_| Error::Failed)?;
        if actual != Some(self.row.original.status) {
            return Err(Error::Stale);
        }
        self.require_current()?;
        let selected = {
            let book = self.book.upgrade().ok_or(Error::Unavailable)?;
            let entries = book.try_lock().map_err(|_| Error::Busy)?;
            entries
                .iter()
                .find(|entry| {
                    Weak::ptr_eq(&Arc::downgrade(&entry.runtime), &self.row.original.runtime)
                        && entry.invocation == self.row.original.invocation
                        && entry.request == self.row.original.request
                })
                .cloned()
                .ok_or(Error::Unavailable)?
        }; // original book/guard leave before issuer preparation
        let keys = prepare(&selected.owner, &self.command.as_ref().unwrap().ui.command);
        let cell = selected.runtime.clone();
        drop(selected); // no issuer enters either runtime writer or SDK/IO
        let keys = keys.map_err(|_| Error::Unavailable)?;
        self.require_current()?;
        Ok((cell, keys))
    }

    /// Borrow the genuine whole runtime only after original lookup and issuer
    /// preparation have completed outside all native and Root guards.
    pub fn finish(self) -> Result<NativeTextMemberJob<T, R>, V2MemberSaveRefusal> {
        let prepared = self.prepare_keys()?;
        self.finish_prepared(prepared)
    }

    /// Run an incoming join on this exact native founder runtime. The
    /// caller owns the request bytes and invokes this job outside UI guards.
    pub fn accept_join_request(
        self,
        bytes: &[u8],
        negative: &RequestCancellation,
    ) -> Result<crate::store::NativeTextSavedJoinResponse, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Error;
        if negative.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let packet =
            catcoms_sync::V2JoinRequestPacketData::decode(bytes).map_err(|_| Error::Failed)?;
        let (keys, cuts) = self.prepare_join_keys()?;
        self.finish_prepared(keys)?
            .run_join_request(&packet, cuts, negative)
    }

    /// Remove an exact selected admission on this original founder row. The
    /// caller supplies the admitted incarnation, never a replacement signer.
    pub fn remove_governed_admission(
        self,
        target: catcoms_replication::chat::owner_header::OwnerBodyId,
        negative: &RequestCancellation,
    ) -> Result<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64), V2MemberSaveRefusal>
    {
        use V2MemberSaveRefusal as Error;
        if negative.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let (cell, (keys, mut sdk, mut header)) =
            self.prepare_using_issuer(|source, command| {
                use catcoms_rt::NativeChatKeyPurpose as Purpose;
                let cut = |purpose| source.prepare_cut(command, purpose).map_err(|_| refused());
                Ok((
                    NativeFounderKeys::prepare(source, command)?,
                    cut(Purpose::RemoveSdk)?,
                    cut(Purpose::RemovalHeader)?,
                ))
            })?;
        let mut job = self.finish_prepared((cell, keys))?;
        let mut ends = catcoms_rt::NativeChatKeyEnds::new();
        job.inner
            .require_current()
            .map_err(|_| Error::Unavailable)?;
        let inner = &mut job.inner;
        let body = inner.loan.body.as_mut().ok_or(Error::Unavailable)?;
        let driver = body.driver.as_mut().ok_or(Error::Unavailable)?;
        let result = body.work.remove_governed_member_native(
            driver,
            target,
            &inner.keys,
            (&mut sdk, &mut header, &mut ends),
            body.rng.as_mut().ok_or(Error::Unavailable)?,
            negative,
        );
        inner.loan.recorded = true;
        if negative.is_cancelled() {
            return Err(Error::Cancelled);
        }
        inner.require_current().map_err(|_| Error::Unavailable)?;
        result.map_err(|error| match error {
            AppError::Io(_) => Error::Busy,
            _ => Error::Failed,
        })
    }

    fn prepare_join_keys(&self) -> Result<JoinKeys<T, R>, V2MemberSaveRefusal> {
        use catcoms_rt::NativeChatKeyPurpose as Purpose;
        let (cell, (keys, cuts)) = self.prepare_using_issuer(|source, command| {
            let cut = |purpose| source.prepare_cut(command, purpose).map_err(|_| refused());
            Ok((
                NativeFounderKeys::prepare(source, command)?,
                [
                    cut(Purpose::AddSdk)?,
                    cut(Purpose::GoverningHeader)?,
                    cut(Purpose::JoinWelcome)?,
                ],
            ))
        })?;
        Ok(((cell, keys), cuts))
    }

    fn finish_prepared(
        mut self,
        (cell, keys): MemberKeys<T, R>,
    ) -> Result<NativeTextMemberJob<T, R>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Error;
        let body = {
            let mut state = cell.state.try_lock().map_err(|_| Error::Busy)?;
            if state.retiring || !self.scope.is_open() {
                return Err(Error::Unavailable);
            }
            if state.running || state.poisoned {
                return Err(Error::Busy);
            }
            let body = state.body.take().ok_or(Error::Unavailable)?;
            state.running = true;
            body
        };
        // Arm owning restoration immediately after the infallible body move.
        // Any later refusal drops this whole loan only outside native guards.
        let job = NativeTextMemberJob {
            inner: NativeTextCreationJob {
                loan: RuntimeLoan {
                    body: Some(body),
                    cell,
                    recorded: false,
                },
                keys,
                command: self.command.take().unwrap(),
            },
        };
        let body = job.inner.loan.body.as_ref().unwrap();
        let ready = match (&body.driver, &body.recovered_founder) {
            (Some(driver), None) => !self.row.original.recovered_founder && driver.is_ready_data(),
            (None, Some(_)) => self.row.original.recovered_founder,
            _ => false,
        };
        if !ready
            || !body
                .selection
                .as_ref()
                .is_some_and(|row| self.row.same_original(row))
        {
            return Err(Error::Unavailable);
        }
        Ok(job)
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextMemberJob<T, R> {
    /// Blocking phase one only. The caller owns/pays borrowed request DATA and
    /// supplies its complete original account+row cancellation. A returned job
    /// never crosses a UI-commit await: body restoration and key Ends leave on
    /// this consuming call before native can acquire final commit custody.
    pub fn prepare_member_save(
        mut self,
        request: V2MemberSignRequest<'_>,
        negative: &RequestCancellation,
    ) -> Result<V2MemberSaveOutcome, V2MemberSaveRefusal> {
        if negative.is_cancelled() {
            return Err(V2MemberSaveRefusal::Cancelled);
        }
        self.inner
            .require_current()
            .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
        let inner = &mut self.inner;
        let body = inner
            .loan
            .body
            .as_mut()
            .ok_or(V2MemberSaveRefusal::Unavailable)?;
        let rng = body.rng.as_mut().ok_or(V2MemberSaveRefusal::Unavailable)?;
        let result = if let Some(founder) = body.recovered_founder.as_mut() {
            let clock = body
                .clock
                .as_deref()
                .ok_or(V2MemberSaveRefusal::Unavailable)?;
            let draws = body
                .group_rng
                .as_mut()
                .ok_or(V2MemberSaveRefusal::Unavailable)?;
            founder.prepare_member_save_with_keys(
                request,
                inner.keys.member_entry(),
                negative,
                (clock, draws),
                rng,
            )
        } else {
            body.work.prepare_native_member(
                body.driver
                    .as_mut()
                    .ok_or(V2MemberSaveRefusal::Unavailable)?,
                request,
                negative,
                rng,
                &mut inner.keys,
            )
        };
        #[cfg(all(test, any(windows, target_os = "linux")))]
        tests::after_prepare();
        // The actual driver retains all known outcomes before this late check.
        // This flag records return, never an attempt/signature refund.
        inner.loan.recorded = true;
        if negative.is_cancelled() {
            return Err(V2MemberSaveRefusal::Cancelled);
        }
        inner
            .require_current()
            .map_err(|_| V2MemberSaveRefusal::Cancelled)?;
        result
    }
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests;

pub(super) fn invoice<T: MeshTransport, R: CryptoRngCore>() -> usize {
    2 * std::mem::size_of::<NativeTextMemberBudget<T, R>>()
        + 2 * std::mem::size_of::<NativeTextMemberPreparation<T, R>>()
        + 2 * std::mem::size_of::<NativeTextMemberJob<T, R>>()
        + admission::invoice::<T, R>()
        + 2 * std::mem::size_of::<NativeTextMemberHistoryPage>()
        + 2 * std::mem::size_of::<NativeTextMemberHistoryItem<'static>>()
        + 4096 // bounded owned capture/refusal/control envelope overlap
}

impl<T, R> std::fmt::Debug for NativeTextMemberBudget<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTextMemberBudget { .. }")
    }
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for NativeTextMemberPreparation<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTextMemberPreparation { .. }")
    }
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for NativeTextMemberJob<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTextMemberJob { .. }")
    }
}
