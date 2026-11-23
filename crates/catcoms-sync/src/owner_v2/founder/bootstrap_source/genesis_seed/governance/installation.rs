//! Exclusive original MLS installation and disjoint stable source proof.
use super::*;
use catcoms_mls::group::text_current_add::{
    PreparedTextCurrentAddInstallation, RetiredTextCurrentAdd,
};
use catcoms_replication::chat::{
    owner_facts::OwnerFactId, V2GeneralCandidate, V2GeneralPrefixVerifier, V2GeneralRetiredData,
    V2HostSelectionExpectation,
};

/// Original SDK pair and its candidate remain exclusively borrowed. The Host
/// supplies its already-covered selection while retaining the real Current.
pub struct PreparedV2FounderAddInstall<'a> {
    mls: PreparedTextCurrentAddInstallation<'a>,
    negative: V2FounderGovernanceNegative<'a>,
    after: V2InstalledFounderAddData,
    installed: &'a mut Option<V2InstalledFounderAddData>,
    installed_invite: &'a mut Option<[u8; 16]>,
    invite_nonce: [u8; 16],
    founder: OwnerFactId,
    member: &'a mut Option<V2AuthenticatedAdmission>,
    core_retired: &'a mut Option<V2GeneralRetiredData>,
    sdk_retired: &'a mut Option<RetiredTextCurrentAdd>,
}

/// Owns the original completed operation and both returned retired graphs.
/// Drop only after the Host has left the Current and native writers.
#[must_use = "retire the original Add custody outside Current and native writers"]
pub struct RetiredV2FounderAddInstall {
    _original: Box<PendingAdd>,
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2FounderGovernanceNode<T, R> {
    /// Preflight the exclusive SDK swap outside writers. The loan returned
    /// alongside it comes from the same original seed's immutable fields.
    pub fn prepare_install_data(
        &mut self,
    ) -> Result<(V2GenesisSourceFields<'_>, PreparedV2FounderAddInstall<'_>), SyncError> {
        self.require_sdk_current_data()?;
        self.signed_welcome_data()?;
        let invite_nonce = self
            .pending
            .as_ref()
            .ok_or_else(unavailable)?
            .request
            .invite_nonce();
        let invite_slot = self
            .available_invite_slot(invite_nonce)
            .ok_or_else(unavailable)?;
        let p = self.pending.as_deref_mut().ok_or_else(unavailable)?;
        if p.phase != Phase::Materialized
            || p.retired_sdk.is_some()
            || p.retired_core.is_some()
            || self.member.is_some() != self.installed.is_some()
        {
            return Err(unavailable());
        }
        let result = p
            .cell
            .as_ref()
            .ok_or_else(unavailable)?
            .completed_result_data()
            .map_err(|_| unavailable())?;
        let header = p.header.as_ref().ok_or_else(unavailable)?;
        let after = V2InstalledFounderAddData {
            epoch: result.after_epoch_data(),
            authenticator: *result.after_authenticator_data(),
            header: header.id(),
            state: header.spec().state,
        };
        let SeedBody {
            source,
            context,
            genesis,
            metadata,
            device,
            group,
            identity,
        } = &mut *self.node.seed.body;
        let founder = genesis.admission.admission_id();
        let fields = V2GenesisSourceFields {
            source: *source,
            context,
            genesis,
            metadata,
            identity,
        };
        let negative = V2FounderGovernanceNegative {
            source: *source,
            context,
            scope: &metadata.scope,
            parts: StableParts {
                warm: self.node._parts.body.warm.as_ref(),
                fresh: self.node._parts.body.fresh.as_ref(),
            },
        };
        negative.require_live_data()?;
        let mls = PreparedTextCurrentAdd::prepare_install_into(&mut p.cell, device, group)
            .map_err(|_| unavailable())?;
        Ok((
            fields,
            PreparedV2FounderAddInstall {
                mls,
                negative,
                after,
                installed: &mut self.installed,
                installed_invite: &mut self.installed_invites[invite_slot],
                invite_nonce,
                founder,
                member: &mut self.member,
                core_retired: &mut p.retired_core,
                sdk_retired: &mut p.retired_sdk,
            },
        ))
    }

    pub fn take_retired_install_data(&mut self) -> Result<RetiredV2FounderAddInstall, SyncError> {
        let p = self.pending.as_ref().ok_or_else(unavailable)?;
        if p.retired_core.is_none() || p.retired_sdk.is_none() {
            return Err(unavailable());
        }
        Ok(RetiredV2FounderAddInstall {
            _original: self.pending.take().expect("checked original retired Add"),
        })
    }
}

impl PreparedV2FounderAddInstall<'_> {
    pub fn expected_data(&self) -> V2InstalledFounderAddData {
        self.after
    }

    pub fn check_host_selection_data(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        candidate: &V2GeneralCandidate,
    ) -> Result<(), SyncError> {
        self.negative.require_live_data()?;
        self.original_founder(verifier)?;
        let selected =
            V2HostSelectionExpectation::from_trusted_host(self.after.header, self.after.state);
        verifier
            .check_host_selection_data(&selected, candidate)
            .map_err(|_| unavailable())
    }

    /// The Host calls this only after independently validating the covered
    /// head and native admission. A Core refusal retains the original Cell.
    pub fn finish_host_selected(
        self,
        verifier: &mut V2GeneralPrefixVerifier,
        candidate: &mut Option<V2GeneralCandidate>,
    ) -> Result<(), SyncError> {
        self.negative.require_live_data()?;
        let founder = self.original_founder(verifier)?;
        let first_member = self.member.is_none().then_some(founder);
        let selected =
            V2HostSelectionExpectation::from_trusted_host(self.after.header, self.after.state);
        let retired = verifier
            .advance_host_selected(&selected, candidate)
            .map_err(|_| unavailable())?;
        // These slots were empty at preflight and remain exclusively loaned.
        // There is no fallible operation between Core success and caching MLS.
        *self.core_retired = Some(retired);
        *self.sdk_retired = Some(self.mls.install());
        *self.installed = Some(self.after);
        *self.installed_invite = Some(self.invite_nonce);
        if let Some(founder) = first_member {
            *self.member = Some(founder);
        }
        Ok(())
    }

    fn original_founder(
        &self,
        verifier: &V2GeneralPrefixVerifier,
    ) -> Result<V2AuthenticatedAdmission, SyncError> {
        let founder = verifier
            .admission(self.founder)
            .map_err(|_| unavailable())?;
        if self
            .member
            .as_ref()
            .is_some_and(|original| !original.same_original(&founder))
        {
            return Err(unavailable());
        }
        Ok(founder)
    }
}
