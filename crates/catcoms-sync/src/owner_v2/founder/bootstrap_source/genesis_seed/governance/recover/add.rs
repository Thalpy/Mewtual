//! The closed recovered founder's private Add candidate.
use super::*;
use catcoms_mls::{
    group::text_current_add::{PreparedTextCurrentAddInstallation, RetiredTextCurrentAdd},
    persist::text_current_add::TextCurrentAddLimits,
};
use catcoms_replication::chat::{
    join_bindings::WelcomeBindingSpecData, owner_state::transition::ReplayBody, V2GeneralCandidate,
    V2GeneralPrefixVerifier, V2GeneralRetiredData, V2HostSelectionExpectation,
    V2LiveMemberObservation,
};
use catcoms_rt::Clock;

catcoms_rt::redacted_debug! {
    PreparedV2RecoveredAddInstall<'_>;
    V2RecoveredFounderAddSigning<'_>;
}

/// Exclusive recovery-side SDK/Core install loan. The host must prove that
/// its physical selector chose this same covered transition before using it.
pub struct PreparedV2RecoveredAddInstall<'a> {
    mls: PreparedTextCurrentAddInstallation<'a>,
    live_parent: V2LiveMemberObservation,
    founder: OwnerFactId,
    member: &'a V2AuthenticatedAdmission,
    expected: &'a mut V2InstalledFounderAddData,
    after: V2InstalledFounderAddData,
    core_retired: &'a mut Option<V2GeneralRetiredData>,
    sdk_retired: &'a mut Option<RetiredTextCurrentAdd>,
}

impl PreparedV2RecoveredAddInstall<'_> {
    pub fn expected_data(&self) -> V2InstalledFounderAddData {
        self.after
    }

    fn checked_selection(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        candidate: &V2GeneralCandidate,
    ) -> Result<V2HostSelectionExpectation, SyncError> {
        let founder = verifier
            .admission(self.founder)
            .map_err(|_| unavailable())?;
        if !self.live_parent.is_live_cached() || !self.member.same_original(&founder) {
            return Err(unavailable());
        }
        let selection =
            V2HostSelectionExpectation::from_trusted_host(self.after.header, self.after.state);
        verifier
            .check_host_selection_data(&selection, candidate)
            .map_err(|_| unavailable())?;
        Ok(selection)
    }

    pub fn check_host_selection_data(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        candidate: &V2GeneralCandidate,
    ) -> Result<(), SyncError> {
        self.checked_selection(verifier, candidate).map(drop)
    }

    /// After the real durable host selection, install both exact successors.
    /// A Core refusal leaves the original MLS graph and completed Cell owned.
    pub fn finish_host_selected(
        self,
        verifier: &mut V2GeneralPrefixVerifier,
        candidate: &mut Option<V2GeneralCandidate>,
    ) -> Result<(), SyncError> {
        let selected =
            self.checked_selection(verifier, candidate.as_ref().ok_or_else(unavailable)?)?;
        let retired = verifier
            .advance_host_selected(&selected, candidate)
            .map_err(|_| unavailable())?;
        // All remaining slots are exclusively borrowed. No fallible work
        // can leave the selected verifier ahead of the recovered SDK pair.
        *self.core_retired = Some(retired);
        *self.sdk_retired = Some(self.mls.install());
        *self.expected = self.after;
        Ok(())
    }
}

/// Borrowed native key tails for the one recovered Add. The host independently
/// holds its original covering journal and current physical writer.
pub struct V2RecoveredFounderAddSigning<'a> {
    pending: &'a mut PendingAdd,
    device: &'a MlsDevice,
    signer: [u8; 32],
    selected: V2LiveMemberObservation,
}

impl V2RecoveredFounderAddSigning<'_> {
    pub fn sign_header_native(&mut self) -> Result<(), SyncError> {
        if !self.selected.is_live_cached() {
            return Err(unavailable());
        }
        self.pending.sign_header_native(self.device, self.signer)
    }

    pub fn sign_welcome_native(&mut self) -> Result<(), SyncError> {
        if !self.selected.is_live_cached() {
            return Err(unavailable());
        }
        self.pending.sign_welcome_native(self.device, self.signer)
    }
}

impl PreparedV2FounderRecoveredAdd {
    fn require_original_add_parent(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        source: &OpenedBootstrapSourceData,
    ) -> Result<(), SyncError> {
        self.bound_member(verifier)?;
        if source.source_id_data() != self.source
            || !self.group.is_designated_committer(&self.device)
        {
            return Err(unavailable());
        }
        Ok(())
    }

    /// Capture the original predecessor's MLS exporter before it retires.
    /// The owning journal binds this verifier to the exact selected step.
    pub fn recovered_add_fetch_verifier_data(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        source: &OpenedBootstrapSourceData,
    ) -> Result<[u8; 32], SyncError> {
        self.require_original_add_parent(verifier, source)?;
        Ok(crate::V2MemberAddRequestData::for_selected_group(
            &self.group,
            &self.device,
            verifier.replay_data(),
        )?
        .verifier_data())
    }

    pub fn has_recovered_add_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Refresh a never-entered native stage through the exact same pending
    /// request after an independently proven old UI batch has retired.
    pub fn prepare_recovered_add_native_stage(
        &mut self,
        stage: V2FounderAddNativeStage,
        cut: NativeChatPreparedKeyCut,
    ) -> Result<(), NativeChatPreparedKeyCut> {
        let Some(pending) = self.pending.as_mut() else {
            return Err(cut);
        };
        pending.prepare_native_stage(stage, cut)
    }

    /// Pure admission preflight against the actual recovered graph and live
    /// original founder Admission. The native host must separately prove that
    /// the invitation is selected in its journal and has never been consumed.
    /// This performs no private SDK work, signing, selection or publication.
    pub fn preflight_recovered_add_package_data(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        source: &OpenedBootstrapSourceData,
        request: V2FounderAddRequestData<'_>,
        clock: &dyn Clock,
        limits: TextCurrentAddLimits,
    ) -> Result<(), SyncError> {
        self.require_original_add_parent(verifier, source)?;
        if self.pending.is_some() {
            return Err(unavailable());
        }
        let (policy, _, _) = request.verify_recovered(source)?;
        drop(policy);
        request.check_live_package((&self.device, &self.group), clock.now_ms(), limits)?;
        self.require_original_add_parent(verifier, source)
    }

    /// Retain one paid candidate on the actual recovered native Source.
    /// The host must cover the exact governing request before preparing and
    /// the distinct SDK entry before execution.
    pub fn prepare_recovered_add_data(
        &mut self,
        (verifier, source): (&V2GeneralPrefixVerifier, &OpenedBootstrapSourceData),
        request: V2FounderAddRequestData<'_>,
        (original, admission, [sdk_cut, header_cut, welcome_cut]): (
            NativeChatSourceObservation,
            TextCurrentAddAdmission,
            [NativeChatPreparedKeyCut; 3],
        ),
    ) -> Result<(), V2FounderAddPreparationRefusal> {
        let preparation = || {
            self.require_original_add_parent(verifier, source)?;
            if self.pending.is_some()
                || !self
                    .native_origin
                    .as_ref()
                    .is_some_and(|bound| bound.same_original(&original))
                || !admission.supports_routing_data()
            {
                return Err(unavailable());
            }
            for (cut, purpose) in [
                (&sdk_cut, NativeChatKeyPurpose::AddSdk),
                (&header_cut, NativeChatKeyPurpose::GoverningHeader),
                (&welcome_cut, NativeChatKeyPurpose::JoinWelcome),
            ] {
                cut.require_never_entered_data(&original, purpose)
                    .map_err(|_| unavailable())?;
                cut.preflight(&original, purpose)
                    .map_err(|_| unavailable())?;
            }
            FrozenAddRequest::prepare_recovered(source, self.expected, request)
        };
        let Ok(request) = preparation() else {
            return Err(Box::new((
                original,
                admission,
                sdk_cut,
                header_cut,
                welcome_cut,
            )));
        };
        self.pending = Some(Box::new(PendingAdd::new(
            request,
            (original, admission, [sdk_cut, header_cut, welcome_cut]),
        )));
        Ok(())
    }

    /// Execute only the retained candidate. The recovered graph stays
    /// selected. Cache the exact result and custody before any late check.
    pub fn execute_recovered_add_native(
        &mut self,
        (verifier, source): (&V2GeneralPrefixVerifier, &OpenedBootstrapSourceData),
        request: V2FounderAddRequestData<'_>,
        runtime: (&dyn Clock, &mut impl CryptoRngCore),
    ) -> Result<(), SyncError> {
        self.require_original_add_parent(verifier, source)?;
        let pending = self.pending.as_mut().ok_or_else(unavailable)?;
        pending.execute_sdk(request, (&self.device, &self.group), runtime)?;
        self.require_original_add_parent(verifier, source)
    }

    /// Exact already-completed SDK data for the host's covering transaction.
    /// It carries no authority to sign, select or publish.
    pub fn completed_recovered_add_data(
        &self,
    ) -> Result<CompletedTextCurrentAddData<'_>, SyncError> {
        self.pending
            .as_ref()
            .and_then(|pending| pending.cell.as_ref())
            .ok_or_else(unavailable)?
            .completed_result_data()
            .map_err(|_| unavailable())
    }

    /// Derive the one unsigned Admission from the completed SDK observation
    /// and the same original selected Core parent. The native host still
    /// covers this exact outcome before it may enter the Header cut.
    pub fn prepare_recovered_admission_data(
        &mut self,
        verifier: &V2GeneralPrefixVerifier,
        source: &OpenedBootstrapSourceData,
    ) -> Result<(), SyncError> {
        self.require_original_add_parent(verifier, source)?;
        self.pending
            .as_mut()
            .ok_or_else(unavailable)?
            .prepare_admission(
                verifier.replay_data(),
                source.view_data().context,
                &source.genesis.profile,
                self.expected.header,
            )?;
        self.require_original_add_parent(verifier, source)
    }

    /// Lend the one completed SDK/unsigned pair to the covering journal.
    /// This can be inspected after a late refusal and confers no key access.
    pub fn recovered_add_outcome_data(&self) -> Result<V2FounderAddOutcomeData<'_>, SyncError> {
        self.pending.as_ref().ok_or_else(unavailable)?.outcome()
    }

    /// Preflight outside writers. The returned loan enters only the actual
    /// covered native signing stage, without any SDK graph comparison there.
    pub fn recovered_add_signing_parts(
        &mut self,
        verifier: &V2GeneralPrefixVerifier,
        source: &OpenedBootstrapSourceData,
    ) -> Result<V2RecoveredFounderAddSigning<'_>, SyncError> {
        self.require_original_add_parent(verifier, source)?;
        let selected = verifier
            .observe_live_member(self.founder)
            .map_err(|_| unavailable())?;
        Ok(V2RecoveredFounderAddSigning {
            pending: self.pending.as_mut().ok_or_else(unavailable)?,
            device: &self.device,
            signer: self.signer,
            selected,
        })
    }

    /// The exact cached Header signature for the host's raw journal record.
    pub fn raw_recovered_add_header_data(&self) -> Result<&[u8; 64], SyncError> {
        self.pending
            .as_ref()
            .and_then(|pending| pending.raw.as_ref())
            .ok_or_else(unavailable)
    }

    /// Materialize the signed Header without entering a key again and derive
    /// the exact Welcome preimage. Cover the corresponding records before
    /// invoking the separate Welcome key cut.
    pub fn prepare_recovered_welcome_data(&mut self) -> Result<(), SyncError> {
        let pending = self.pending.as_mut().ok_or_else(unavailable)?;
        pending.materialize_header()?;
        pending.prepare_welcome()
    }

    pub fn recovered_add_header_data(&self) -> Result<&SignedOwnerHeader, SyncError> {
        self.pending
            .as_ref()
            .and_then(|pending| pending.header.as_ref())
            .ok_or_else(unavailable)
    }

    pub fn recovered_welcome_intent_data(
        &self,
    ) -> Result<(&WelcomeBindingSpecData, &[u8]), SyncError> {
        self.pending
            .as_ref()
            .ok_or_else(unavailable)?
            .unsigned_welcome()
    }

    pub fn raw_recovered_welcome_data(&self) -> Result<&[u8; 64], SyncError> {
        self.pending.as_ref().ok_or_else(unavailable)?.raw_welcome()
    }

    pub fn materialize_recovered_welcome_data(&mut self) -> Result<(), SyncError> {
        self.pending
            .as_mut()
            .ok_or_else(unavailable)?
            .materialize_welcome()
    }

    /// The same completed response and signed Header for archival. Only the
    /// host's actual selected disk Head can authorize releasing the packet.
    pub fn recovered_join_response_data(
        &self,
    ) -> Result<(&SignedOwnerHeader, crate::V2JoinResponseData<'_>), SyncError> {
        let pending = self.pending.as_ref().ok_or_else(unavailable)?;
        Ok((
            pending.header.as_ref().ok_or_else(unavailable)?,
            pending.join_response()?,
        ))
    }

    /// Derive the Core successor from the same SDK-backed signed Header.
    pub fn derive_recovered_add_selection_data(
        &self,
        verifier: &mut V2GeneralPrefixVerifier,
        source: &OpenedBootstrapSourceData,
    ) -> Result<V2GeneralCandidate, SyncError> {
        self.require_original_add_parent(verifier, source)?;
        let pending = self.pending.as_ref().ok_or_else(unavailable)?;
        let outcome = pending.outcome()?;
        verifier
            .derive_successor(
                pending.header.as_ref().ok_or_else(unavailable)?,
                ReplayBody::Admission(outcome.admission_data()),
                outcome.unsigned_data().image_data(),
            )
            .map_err(|_| unavailable())
    }

    pub fn prepare_recovered_add_install(
        &mut self,
        verifier: &V2GeneralPrefixVerifier,
        source: &OpenedBootstrapSourceData,
    ) -> Result<PreparedV2RecoveredAddInstall<'_>, SyncError> {
        self.require_original_add_parent(verifier, source)?;
        let live_parent = verifier
            .observe_live_member(self.founder)
            .map_err(|_| unavailable())?;
        let member = self.member.as_ref().ok_or_else(unavailable)?;
        let pending = self.pending.as_deref_mut().ok_or_else(unavailable)?;
        if pending.phase != Phase::Materialized
            || pending.retired_sdk.is_some()
            || pending.retired_core.is_some()
        {
            return Err(unavailable());
        }
        pending.join_response()?;
        let outcome = pending.outcome()?;
        let header = pending.header.as_ref().ok_or_else(unavailable)?;
        let after = V2InstalledFounderAddData {
            epoch: outcome.result_data().after_epoch_data(),
            authenticator: *outcome.result_data().after_authenticator_data(),
            header: header.id(),
            state: header.spec().state,
        };
        let mls = PreparedTextCurrentAdd::prepare_install_into(
            &mut pending.cell,
            &mut self.device,
            &mut self.group,
        )
        .map_err(|_| unavailable())?;
        Ok(PreparedV2RecoveredAddInstall {
            mls,
            live_parent,
            founder: self.founder,
            member,
            expected: &mut self.expected,
            after,
            core_retired: &mut pending.retired_core,
            sdk_retired: &mut pending.retired_sdk,
        })
    }

    pub fn take_retired_recovered_add_install(
        &mut self,
    ) -> Result<RetiredV2FounderAddInstall, SyncError> {
        installation::take_retired_add(&mut self.pending)
    }

    /// Release only a proved closed UI generation's cuts. The original
    /// candidate and outcome remain retained for covering IO or exact retry.
    pub fn retire_recovered_add_for_closed_ui(
        &mut self,
        closed: &[Option<&catcoms_rt::NativeChatKeyDrain>],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<RetiredV2FounderAddNativeCustody>, SyncError> {
        match self.pending.as_mut() {
            Some(pending) => pending.retire_for_closed_ui(closed, rng),
            None => Ok(None),
        }
    }

    /// On permanent Source shutdown, dispose the same retained operation
    /// before waiting for the Ends the operation itself owns. The host first
    /// settles all in-flight durable work and retains its journal separately.
    pub fn discard_recovered_add_for_closed_source(
        &mut self,
        closed: &catcoms_rt::NativeChatKeyDrain,
    ) -> Result<(), SyncError> {
        if let Some(operation) = self.pending.as_ref() {
            closed
                .require_closed_source(&operation.original)
                .map_err(|_| unavailable())?;
            drop(self.pending.take());
        }
        Ok(())
    }
}
