//! Native founder publication of the original durable ordinary outbox debt.
use super::*;
use crate::store::persistence::v2_member_outbox::MemberDebtBatch;
use crate::store::persistence::v2_member_transport::{FounderDebtOperation, FounderDebtOutcome};
use catcoms_rt::PublishSubmission;
use catcoms_sync::owner_v2::PreparedV2MemberSubmission;

struct Custody {
    _work: SourceWork,
    _end: retirement::PreparationEnd,
    _fee: CatalogReadReservation,
}

impl std::fmt::Debug for Custody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FounderPublicationCustody")
            .finish_non_exhaustive()
    }
}

#[must_use = "the saved message remains pending until the original mesh submits it"]
pub struct NativeTextFounderMemberPublication<T: MeshTransport, R: CryptoRngCore> {
    submission: PreparedV2MemberSubmission,
    receipt: NativeTextFounderMemberPublicationReceipt<T, R>,
    signal: RequestCancellation,
}

#[must_use = "the acknowledgement must checkpoint the original outbox"]
pub struct NativeTextFounderMemberPublicationReceipt<T: MeshTransport, R: CryptoRngCore> {
    batch: MemberDebtBatch,
    row: NativeTextRuntimeSelection<T, R>,
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextFounderMemberPublication<T, R> {
    pub async fn submit(
        self,
    ) -> Result<Option<NativeTextFounderMemberPublicationReceipt<T, R>>, catcoms_sync::SyncError>
    {
        let Self {
            submission,
            mut receipt,
            signal,
        } = self;
        match submission.submit(signal).await? {
            PublishSubmission::Duplicate => Ok(None),
            PublishSubmission::Submitted => {
                receipt
                    .batch
                    .record_submitted(0)
                    .map_err(|_| catcoms_sync::SyncError::JoinRejected)?;
                Ok(Some(receipt))
            }
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextMemberPreparation<T, R> {
    pub fn prepare_founder_member_publication(
        mut self,
        caller: RequestCancellation,
    ) -> Result<Option<NativeTextFounderMemberPublication<T, R>>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let (cell, mut keys) = self.prepare_keys()?;
        let source = keys.member_entry().1.observe_current();
        let signal =
            connected::constrain_founder_transport(self.scope.clone(), source, caller, None);
        if signal.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        let row = self.row.clone();
        let work = self.work.take().ok_or(Refusal::Unavailable)?;
        let end = self._end.take().ok_or(Refusal::Unavailable)?;
        let result = self
            .finish_prepared((cell, keys))?
            .run_founder_member_debt(FounderDebtOperation::Prepare, &signal)?;
        match result {
            FounderDebtOutcome::Prepared {
                submission,
                batch,
                fee,
            } => {
                let signal = signal.retain_custody(Arc::new(Custody {
                    _work: work,
                    _end: end,
                    _fee: fee,
                }));
                Ok(Some(NativeTextFounderMemberPublication {
                    submission,
                    receipt: NativeTextFounderMemberPublicationReceipt { batch, row },
                    signal,
                }))
            }
            FounderDebtOutcome::Idle => Ok(None),
            FounderDebtOutcome::Checkpointed(_) => Err(Refusal::Failed),
        }
    }

    pub fn checkpoint_founder_member_publication(
        self,
        receipt: NativeTextFounderMemberPublicationReceipt<T, R>,
        caller: &RequestCancellation,
    ) -> Result<u64, V2MemberSaveRefusal> {
        if !self.row.same_original(&receipt.row) {
            return Err(V2MemberSaveRefusal::Stale);
        }
        match self
            .finish()?
            .run_founder_member_debt(FounderDebtOperation::Checkpoint(receipt.batch), caller)?
        {
            FounderDebtOutcome::Checkpointed(front) => Ok(front),
            _ => Err(V2MemberSaveRefusal::Failed),
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextMemberJob<T, R> {
    fn run_founder_member_debt(
        mut self,
        operation: FounderDebtOperation,
        caller: &RequestCancellation,
    ) -> Result<FounderDebtOutcome, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        self.inner
            .require_current()
            .map_err(|_| Refusal::Unavailable)?;
        let inner = &mut self.inner;
        let body = inner.loan.body.as_mut().ok_or(Refusal::Unavailable)?;
        let result = body.work.run_founder_member_debt(
            body.driver.as_mut().ok_or(Refusal::Unavailable)?,
            operation,
            caller,
            body.rng.as_mut().ok_or(Refusal::Unavailable)?,
            &mut inner.keys,
        );
        inner.loan.recorded = true;
        self.inner
            .require_current()
            .map_err(|_| Refusal::Cancelled)?;
        if caller.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        result
    }
}
