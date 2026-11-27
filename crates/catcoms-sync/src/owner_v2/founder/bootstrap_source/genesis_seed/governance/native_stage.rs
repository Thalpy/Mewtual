//! Finite native work versus retained closed DATA. These APIs do not issue
//! Root cancellation/saved/current authority. In particular a native key drain
//! is not actual SDK/body/physical-IO destruction or a whole Source close.
use super::*;

/// A local stage selector, never a serialized WebView permission or Root proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V2FounderAddNativeStage {
    Sdk,
    GoverningHeader,
    Welcome,
}
impl V2FounderAddNativeStage {
    fn purpose(self) -> NativeChatKeyPurpose {
        match self {
            Self::Sdk => NativeChatKeyPurpose::AddSdk,
            Self::GoverningHeader => NativeChatKeyPurpose::GoverningHeader,
            Self::Welcome => NativeChatKeyPurpose::JoinWelcome,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AdmissionState {
    Unstarted,
    Attempting,
    KnownLocalRefusal,
    Admitted,
}

/// Private witness from the ACTUAL native enter call. Neither missing raw nor
/// a protocol phase can manufacture it. Attempt accounting survives reauth.
pub(super) struct NativeStageAdmission {
    state: AdmissionState,
    attempts: u64,
}
impl NativeStageAdmission {
    pub(super) fn new() -> Self {
        Self {
            state: AdmissionState::Unstarted,
            attempts: 0,
        }
    }
    pub(super) fn may_enter(&self) -> bool {
        self.state == AdmissionState::Unstarted
    }
    fn eligible(&self) -> bool {
        matches!(
            self.state,
            AdmissionState::Unstarted | AdmissionState::KnownLocalRefusal
        )
    }
    fn can_retire(
        &self,
        cut: &NativeChatPreparedKeyCut,
        source: &NativeChatSourceObservation,
        purpose: NativeChatKeyPurpose,
    ) -> bool {
        self.state == AdmissionState::KnownLocalRefusal
            || (self.state == AdmissionState::Unstarted
                && cut.require_never_entered_data(source, purpose).is_ok())
    }
    pub(super) fn enter(
        &mut self,
        cut: &mut NativeChatPreparedKeyCut,
        source: &NativeChatSourceObservation,
        purpose: NativeChatKeyPurpose,
        ends: &mut NativeChatKeyEnds,
    ) -> Result<(), SyncError> {
        if self.state != AdmissionState::Unstarted {
            return Err(unavailable());
        }
        self.attempts = self.attempts.checked_add(1).ok_or_else(unavailable)?;
        // BEFORE calling RT: an unwind must stay terminal, rather than a retry
        // receiving AlreadyEntered and laundering it into a refusal witness.
        self.state = AdmissionState::Attempting;
        #[cfg(test)]
        test_cut(TestCut::NativeAdmissionAttempting);
        match cut.enter(source, purpose, ends) {
            Ok(()) => {
                self.state = AdmissionState::Admitted;
                Ok(())
            }
            Err(
                catcoms_rt::NativeChatKeyRefusal::Busy
                | catcoms_rt::NativeChatKeyRefusal::Unavailable,
            ) => {
                self.state = AdmissionState::KnownLocalRefusal;
                Err(unavailable())
            }
            // Foreign/WrongPurpose/AlreadyEntered are invariant failures, not
            // evidence that a fresh native admission is safe.
            Err(_) => Err(unavailable()),
        }
    }
    fn reauthorize(&mut self) {
        debug_assert!(self.eligible());
        self.state = AdmissionState::Unstarted; // attempts are never reset
    }

    /// Reopen only a locally refused native edge. Its owning operation must
    /// still retain the original request, source and unentered primitive.
    pub(super) fn retry_local_refusal(
        &mut self,
        cut: &NativeChatPreparedKeyCut,
        source: &NativeChatSourceObservation,
        purpose: NativeChatKeyPurpose,
    ) -> Result<(), SyncError> {
        if self.state == AdmissionState::KnownLocalRefusal {
            cut.require_never_entered_data(source, purpose)
                .map_err(|_| unavailable())?;
            cut.preflight(source, purpose).map_err(|_| unavailable())?;
            self.reauthorize();
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn attempts_for_test(&self) -> u64 {
        self.attempts
    }
}

/// Dispose OUTSIDE every native/Root/Core writer. Metadata pins are retired
/// with their actual End batch, rather than retaining obsolete UI-generation
/// payments for the lifetime of an inert closed SDK snapshot.
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use catcoms_sync::owner_v2::PreparedV2FounderGovernanceNode;
/// fn ignore(node: &mut PreparedV2FounderGovernanceNode<(), ()>) {
///     node.retire_closed_add_native_data().unwrap();
/// }
/// ```
///
/// ```
/// use catcoms_sync::owner_v2::PreparedV2FounderGovernanceNode;
/// fn retire_outside_writers(node: &mut PreparedV2FounderGovernanceNode<(), ()>) {
///     let retired = node.retire_closed_add_native_data().unwrap();
///     drop(retired);
/// }
/// ```
#[must_use = "retain and dispose native custody outside every native/Root/Core writer"]
pub struct RetiredV2FounderAddNativeCustody {
    _sdk: Option<NativeChatPreparedKeyCut>,
    _header: Option<NativeChatPreparedKeyCut>,
    _welcome: Option<NativeChatPreparedKeyCut>,
    // LAST: displaced cuts are gone before counter retirement can wake the
    // native close job. Its own metadata pins survive through that retirement.
    _ends: NativeChatKeyEnds,
}
impl std::fmt::Debug for RetiredV2FounderAddNativeCustody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetiredV2FounderAddNativeCustody")
            .finish_non_exhaustive()
    }
}

impl<T, R: CryptoRngCore> PreparedV2FounderGovernanceNode<T, R> {
    /// UI lock keeps the Source and the original request alive. Only retire
    /// custody from the actual closed generation's batch. A partial Cell
    /// first finishes that same admitted SDK unit outside writers.
    pub fn retire_add_for_closed_ui(
        &mut self,
        closed: &[Option<&catcoms_rt::NativeChatKeyDrain>],
    ) -> Result<Option<RetiredV2FounderAddNativeCustody>, SyncError> {
        let Some(pending) = self.pending.as_mut() else {
            return Ok(None);
        };
        pending.retire_for_closed_ui(closed, &mut self.node._parts.body._rng)
    }
}

impl PendingAdd {
    pub(super) fn retire_for_closed_ui(
        &mut self,
        closed: &[Option<&catcoms_rt::NativeChatKeyDrain>],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<RetiredV2FounderAddNativeCustody>, SyncError> {
        let pending = self;
        let cuts = [&pending.sdk_cut, &pending.header_cut, &pending.welcome_cut];
        if !cuts
            .into_iter()
            .filter_map(Option::as_ref)
            .any(|cut| cut.belongs_to_closed_ui(closed))
        {
            return Ok(None);
        }
        if !cuts
            .into_iter()
            .filter_map(Option::as_ref)
            .all(|cut| cut.belongs_to_closed_ui(closed))
            || !pending.ends.all_ui_counters_closed_in(closed)
        {
            return Err(unavailable());
        }
        let phase = pending.phase;
        if phase == Phase::Prepared {
            return pending.retire_unstarted_native().map(Some);
        }
        if phase == Phase::SdkEntered
            && !pending
                .cell
                .as_ref()
                .is_some_and(|cell| cell.completed_result_data().is_ok())
        {
            pending.finish_admitted_sdk(rng)?;
        }
        pending.retire_closed_native().map(Some)
    }
}

impl<T, R: CryptoRngCore> PreparedV2FounderGovernanceNode<T, R> {
    /// Complete ONLY the SAME already-admitted finite SDK unit from its frozen
    /// inputs/time and retained RNG. It neither constructs another Cell nor
    /// enters a new native cut. A real retained source End is mandatory even
    /// after UI closure. Root's cleanup owner must retain this capsule through
    /// all partial/lost-return continuations before waiting on key drainage.
    pub fn finish_admitted_add_data_native(&mut self) -> Result<(), SyncError> {
        self.pending
            .as_mut()
            .ok_or_else(unavailable)?
            .finish_admitted_sdk(&mut self.node._parts.body._rng)
    }
}

impl PendingAdd {
    pub(super) fn finish_admitted_sdk(
        &mut self,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), SyncError> {
        let p = self;
        if p.sdk_continuation_retired
            || p.phase != Phase::SdkEntered
            || !p
                .ends
                .holds_admitted_purpose_data(&p.original, NativeChatKeyPurpose::AddSdk)
        {
            return Err(unavailable());
        }
        p.cell
            .as_mut()
            .ok_or_else(unavailable)?
            .execute_private_with_routing(rng)
            .map_err(|_| unavailable())?;
        // Actual routing/provider/output closure, not a caller flag or a
        // previously spent cut. Cache phase before any hook/lost return.
        p.cell
            .as_ref()
            .ok_or_else(unavailable)?
            .completed_result_data()
            .map_err(|_| unavailable())?;
        p.phase = Phase::Complete;
        #[cfg(test)]
        test_cut(TestCut::SdkComplete);
        Ok(())
    }
}

impl<T, R> PreparedV2FounderGovernanceNode<T, R> {
    /// Permanent Source shutdown only. The owning runtime must first finish
    /// admitting/dispatching its finite disk work. Dispose the entire pending
    /// operation before waiting on the actual Ends retained by that operation.
    /// Its durable controls remain available to independent recovery.
    pub fn discard_add_for_closed_source(
        &mut self,
        closed: &catcoms_rt::NativeChatKeyDrain,
    ) -> Result<(), SyncError> {
        let Some(operation) = self.pending.as_ref() else {
            return Ok(());
        };
        closed
            .require_closed_source(&operation.original)
            .map_err(|_| unavailable())?;
        self.node.seed.body.metadata.scope.retire();
        drop(self.pending.take());
        Ok(())
    }

    /// Move finite native custody after the Cell has fully closed or the
    /// successful installation has cached both returned retired owners.
    /// Partial resumable SDK work must keep its original End; there is no
    /// inference from phase alone or another UI generation. The closed Cell
    /// remains historical DATA, while SDK continuation is permanently fenced.
    ///
    /// Root must independently own the real stage/IO worker and dispose this
    /// returned envelope outside writers. Key drain alone supplies no Root
    /// save, whole Source acknowledgement or response-release permission.
    pub fn retire_closed_add_native_data(
        &mut self,
    ) -> Result<RetiredV2FounderAddNativeCustody, SyncError> {
        self.pending
            .as_mut()
            .ok_or_else(unavailable)?
            .retire_closed_native()
    }

    /// Retire cancelled/pre-effect native metadata ONLY before any SDK Cell
    /// exists. Unknown native entry is not recoverable from phase alone.
    /// The same request/payment stays owned and all prior debits remain spent.
    pub fn retire_unstarted_add_native_data(
        &mut self,
    ) -> Result<RetiredV2FounderAddNativeCustody, SyncError> {
        self.pending
            .as_mut()
            .ok_or_else(unavailable)?
            .retire_unstarted_native()
    }

    /// Bind a fresh, genuinely never-entered SAME-source cut after displaced
    /// custody has retired outside writers. Root MUST freshly authenticate the
    /// exact operation, covered intent/outcome, current parent and source.
    /// This DATA seam grants none of those permissions and refunds no attempt.
    pub fn prepare_add_native_stage_data(
        &mut self,
        stage: V2FounderAddNativeStage,
        cut: NativeChatPreparedKeyCut,
    ) -> Result<(), NativeChatPreparedKeyCut> {
        let Some(p) = self.pending.as_mut() else {
            return Err(cut);
        };
        if cut
            .require_never_entered_data(&p.original, stage.purpose())
            .is_err()
            || cut.preflight(&p.original, stage.purpose()).is_err()
        {
            return Err(cut);
        }
        match stage {
            V2FounderAddNativeStage::Sdk
                if p.phase == Phase::Prepared
                    && p.cell.is_none()
                    && p.sdk_cut.is_none()
                    && p.sdk_may_reauthorize
                    && !p.sdk_continuation_retired
                    && p.sdk_native.eligible() =>
            {
                p.sdk_native.reauthorize();
                p.sdk_may_reauthorize = false;
                p.sdk_cut = Some(cut);
            }
            V2FounderAddNativeStage::GoverningHeader
                if p.phase == Phase::Complete
                    && p.unsigned.is_some()
                    && p.raw.is_none()
                    && p.header_cut.is_none()
                    && p.header_may_reauthorize
                    && p.header_native.eligible() =>
            {
                p.header_native.reauthorize();
                p.header_may_reauthorize = false;
                p.header_cut = Some(cut);
            }
            V2FounderAddNativeStage::Welcome
                if p.phase == Phase::Materialized
                    && p.welcome_cut.is_none()
                    && p.welcome_may_reauthorize
                    && p.welcome_native.eligible() =>
            {
                let Some(w) = p.welcome.as_mut() else {
                    return Err(cut);
                };
                if !w.may_reauthorize_native_data() {
                    return Err(cut);
                }
                p.welcome_native.reauthorize();
                p.welcome_may_reauthorize = false;
                p.welcome_cut = Some(cut);
            }
            _ => return Err(cut),
        }
        Ok(())
    }
}

impl PendingAdd {
    fn retire_closed_native(&mut self) -> Result<RetiredV2FounderAddNativeCustody, SyncError> {
        if self.retired_sdk.is_none() || self.retired_core.is_none() {
            self.cell
                .as_ref()
                .ok_or_else(unavailable)?
                .completed_result_data()
                .map_err(|_| unavailable())?;
        }
        if self.phase == Phase::SdkEntered {
            // A lost Sync return may leave the phase behind the actual Cell.
            self.phase = Phase::Complete;
        }
        self.sdk_continuation_retired = true;
        Ok(self.take_native_custody())
    }

    fn retire_unstarted_native(&mut self) -> Result<RetiredV2FounderAddNativeCustody, SyncError> {
        let cut = self.sdk_cut.as_ref().ok_or_else(unavailable)?;
        if self.phase != Phase::Prepared
            || self.cell.is_some()
            || self.admission.is_none()
            || self.sdk_continuation_retired
            || !self
                .sdk_native
                .can_retire(cut, &self.original, NativeChatKeyPurpose::AddSdk)
        {
            return Err(unavailable());
        }
        self.sdk_may_reauthorize = true;
        Ok(self.take_native_custody())
    }

    fn take_native_custody(&mut self) -> RetiredV2FounderAddNativeCustody {
        record_key_eligibility(self);
        RetiredV2FounderAddNativeCustody {
            _sdk: self.sdk_cut.take(),
            _header: self.header_cut.take(),
            _welcome: self.welcome_cut.take(),
            _ends: std::mem::take(&mut self.ends),
        }
    }
}

fn record_key_eligibility(p: &mut PendingAdd) {
    if let Some(cut) = &p.header_cut {
        // Native eligibility only: pre-SDK cleanup must retain evidence that
        // this future Header cut never entered. The actual completed SDK and
        // frozen preimage are required again at fresh stage authorization.
        p.header_may_reauthorize =
            p.header_native
                .can_retire(cut, &p.original, NativeChatKeyPurpose::GoverningHeader);
    }
    if let Some(cut) = &p.welcome_cut {
        p.welcome_may_reauthorize =
            p.welcome_native
                .can_retire(cut, &p.original, NativeChatKeyPurpose::JoinWelcome);
    }
}
