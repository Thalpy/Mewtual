//! Exclusive founder Remove on the original selected Core and MLS parent.
use super::*;
use catcoms_mls::group::text_current_add::{
    CompletedTextCurrentRemoveData, PreparedTextCurrentRemove,
    PreparedTextCurrentRemoveInstallation, RetiredTextCurrentRemove,
};
use catcoms_replication::chat::{
    owner_header::{OwnerBodyId, MAX_HEADER_SEMANTIC_BYTES},
    owner_state::transition::{ReplayBody, UnsignedRemovalStepData},
    V2AdmissionCloseCompletion, V2FrozenAdmission, V2GeneralCandidate, V2GeneralPrefixVerifier,
    V2GeneralRetiredData, V2HostSelectionExpectation,
};

pub(super) struct PendingRemoval {
    parent: V2InstalledFounderAddData,
    admission: OwnerBodyId,
    subject: [u8; 32],
    original: NativeChatSourceObservation,
    sdk_entry: NativeStageAdmission,
    header_entry: NativeStageAdmission,
    cell: Option<PreparedTextCurrentRemove>,
    unsigned: Option<UnsignedRemovalStepData>,
    raw: Option<[u8; 64]>,
    header: Option<SignedOwnerHeader>,
    retired: Option<(V2GeneralRetiredData, RetiredTextCurrentRemove)>,
}

pub struct V2FounderRemoveOutcomeData<'a> {
    pub result: CompletedTextCurrentRemoveData<'a>,
    pub unsigned: &'a UnsignedRemovalStepData,
}

pub struct V2FounderRemoveSigning<'a> {
    seed: &'a UnboundV2GenesisSeed,
    negative: V2FounderGovernanceNegative<'a>,
    pending: &'a mut PendingRemoval,
}

pub struct PreparedV2FounderRemoveInstall<'a> {
    mls: PreparedTextCurrentRemoveInstallation<'a>,
    negative: V2FounderGovernanceNegative<'a>,
    parent: V2InstalledFounderAddData,
    after: V2InstalledFounderAddData,
    installed: &'a mut Option<V2InstalledFounderAddData>,
    member: &'a V2AuthenticatedAdmission,
    retired: &'a mut Option<(V2GeneralRetiredData, RetiredTextCurrentRemove)>,
}

#[must_use = "retire the displaced Remove custody after leaving the host writers"]
pub struct RetiredV2FounderRemoveInstall {
    original: Box<PendingRemoval>,
}

pub(super) const METADATA_BYTES: usize = UNSIGNED_ADMISSION_DATA_INVOICE
    + 4 * (std::mem::size_of::<PendingRemoval>()
        + std::mem::size_of::<V2FounderRemoveOutcomeData<'static>>()
        + std::mem::size_of::<V2FounderRemoveSigning<'static>>()
        + std::mem::size_of::<PreparedV2FounderRemoveInstall<'static>>()
        + std::mem::size_of::<RetiredV2FounderRemoveInstall>());

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2FounderGovernanceNode<T, R> {
    /// Freeze the target in the original selected slot before any private SDK
    /// work. The host owns the separately covered Remove intent and signer.
    pub fn prepare_removal_data(
        &mut self,
        verifier: &V2GeneralPrefixVerifier,
        admission: OwnerBodyId,
        original: NativeChatSourceObservation,
    ) -> Result<(), SyncError> {
        self.observe_member(verifier, 0)?;
        let parent = self.installed.ok_or_else(unavailable)?;
        if self.pending.is_some() {
            return Err(unavailable());
        }
        if let Some(pending) = &self.removal {
            return pending.require_original(parent, admission, &original);
        }
        let replay = verifier.replay_data();
        let selected = replay.slot(admission).ok_or_else(unavailable)?;
        let subject = selected.image.subject;
        if subject == replay.profile().spec().founder {
            return Err(unavailable());
        }
        self.removal = Some(Box::new(PendingRemoval {
            parent,
            admission,
            subject,
            original,
            sdk_entry: NativeStageAdmission::new(),
            header_entry: NativeStageAdmission::new(),
            cell: None,
            unsigned: None,
            raw: None,
            header: None,
            retired: None,
        }));
        Ok(())
    }

    /// The native owner calls after covering the exact intent and private SDK
    /// Entry. The caller keeps the real key End batch outside host writers.
    pub fn execute_removal_native(
        &mut self,
        verifier: &V2GeneralPrefixVerifier,
        admission: OwnerBodyId,
        original: &NativeChatSourceObservation,
        payment: &mut Option<TextCurrentAddAdmission>,
        cut: &mut NativeChatPreparedKeyCut,
        ends: &mut NativeChatKeyEnds,
    ) -> Result<(), SyncError> {
        self.observe_member(verifier, 0)?;
        let parent = self.installed.ok_or_else(unavailable)?;
        let pending = self.removal.as_deref_mut().ok_or_else(unavailable)?;
        pending.require_original(parent, admission, original)?;
        if pending.unsigned.is_some() {
            return Ok(());
        }
        let seed = &self.node.seed.body;
        if pending.cell.is_none() {
            if payment.is_none() {
                return Err(unavailable());
            }
            let purpose = NativeChatKeyPurpose::RemoveSdk;
            cut.require_never_entered_data(original, purpose)
                .map_err(|_| unavailable())?;
            pending
                .sdk_entry
                .retry_local_refusal(cut, original, purpose)?;
            pending.sdk_entry.enter(cut, original, purpose, ends)?;
            let candidate = PreparedTextCurrentRemove::prepare(
                &seed.device,
                &seed.group,
                pending.subject,
                payment.take().expect("checked removal payment"),
            )
            .map_err(|_| unavailable())?;
            pending.cell = Some(candidate);
        }
        let cell = pending.cell.as_mut().ok_or_else(unavailable)?;
        cell.execute_private(&seed.device, &seed.group)
            .map_err(|_| unavailable())?;
        let result = cell.completed_data().ok_or_else(unavailable)?;
        let digest = *blake3::hash(result.commit_data()).as_bytes();
        let unsigned = verifier
            .replay_data()
            .prepare_removal_data(admission, digest)
            .map_err(|_| unavailable())?;
        if unsigned.body_data().spec().subject != *result.target_data()
            || unsigned.body_data().spec().after_epoch != result.selected_epoch_data().0
        {
            return Err(unavailable());
        }
        pending.unsigned = Some(unsigned);
        Ok(())
    }

    pub fn removal_outcome_data(&self) -> Result<V2FounderRemoveOutcomeData<'_>, SyncError> {
        self.removal.as_deref().ok_or_else(unavailable)?.outcome()
    }

    /// Borrow the exact completed plan alongside the same source fields for
    /// the host to check during its durable Header-attempt admission.
    pub fn removal_signing_parts_data(
        &mut self,
        verifier: &V2GeneralPrefixVerifier,
    ) -> Result<(V2GenesisSourceFields<'_>, V2FounderRemoveSigning<'_>), SyncError> {
        self.observe_member(verifier, 0)?;
        let seed = &self.node.seed;
        let negative = V2FounderGovernanceNegative {
            source: seed.body.source,
            context: &seed.body.context,
            scope: &seed.body.metadata.scope,
            parts: StableParts {
                warm: self.node._parts.body.warm.as_ref(),
                fresh: self.node._parts.body.fresh.as_ref(),
            },
        };
        let pending = self.removal.as_deref_mut().ok_or_else(unavailable)?;
        Ok((
            seed.source_fields(),
            V2FounderRemoveSigning {
                seed,
                negative,
                pending,
            },
        ))
    }

    pub fn raw_removal_signature_data(&self) -> Option<&[u8; 64]> {
        self.removal.as_deref()?.raw.as_ref()
    }

    pub fn materialize_signed_removal_data(&mut self) -> Result<&SignedOwnerHeader, SyncError> {
        let pending = self.removal.as_deref_mut().ok_or_else(unavailable)?;
        if pending.header.is_none() {
            let unsigned = pending.unsigned.as_ref().ok_or_else(unavailable)?;
            let raw = pending.raw.ok_or_else(unavailable)?;
            pending.header = Some(
                SignedOwnerHeader::sign_using(unsigned.header_spec_data().clone(), |bytes| {
                    if bytes != unsigned.signing_bytes_data() {
                        return Err(catcoms_replication::chat::ChatError::Dependency);
                    }
                    Ok(raw)
                })
                .map_err(|_| unavailable())?,
            );
        }
        pending.header.as_ref().ok_or_else(unavailable)
    }

    pub fn derive_removal_selection_data(
        &self,
        verifier: &mut V2GeneralPrefixVerifier,
    ) -> Result<V2GeneralCandidate, SyncError> {
        self.observe_member(verifier, 0)?;
        let pending = self.removal.as_deref().ok_or_else(unavailable)?;
        let plan = pending.outcome()?;
        verifier
            .derive_successor(
                pending.header.as_ref().ok_or_else(unavailable)?,
                ReplayBody::Removal(plan.unsigned.body_data()),
                plan.unsigned.image_data(),
            )
            .map_err(|_| unavailable())
    }

    pub fn prepare_removal_install_data(
        &mut self,
        verifier: &V2GeneralPrefixVerifier,
    ) -> Result<
        (
            V2GenesisSourceFields<'_>,
            PreparedV2FounderRemoveInstall<'_>,
        ),
        SyncError,
    > {
        self.observe_member(verifier, 0)?;
        let pending = self.removal.as_deref_mut().ok_or_else(unavailable)?;
        if Some(pending.parent) != self.installed || pending.retired.is_some() {
            return Err(unavailable());
        }
        let header = pending.header.as_ref().ok_or_else(unavailable)?;
        let (epoch, authenticator) = pending.outcome()?.result.selected_epoch_data();
        let after = V2InstalledFounderAddData {
            epoch,
            authenticator,
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
        let mls = PreparedTextCurrentRemove::prepare_install_into(&mut pending.cell, device, group)
            .map_err(|_| unavailable())?;
        Ok((
            fields,
            PreparedV2FounderRemoveInstall {
                mls,
                negative,
                parent: pending.parent,
                after,
                installed: &mut self.installed,
                member: self.member.as_ref().ok_or_else(unavailable)?,
                retired: &mut pending.retired,
            },
        ))
    }

    pub fn take_retired_removal_data(
        &mut self,
    ) -> Result<RetiredV2FounderRemoveInstall, SyncError> {
        if self
            .removal
            .as_ref()
            .is_none_or(|pending| pending.retired.is_none())
        {
            return Err(unavailable());
        }
        Ok(RetiredV2FounderRemoveInstall {
            original: self.removal.take().expect("installed original Remove"),
        })
    }
}

impl PendingRemoval {
    fn require_original(
        &self,
        parent: V2InstalledFounderAddData,
        admission: OwnerBodyId,
        original: &NativeChatSourceObservation,
    ) -> Result<(), SyncError> {
        if self.parent != parent
            || self.admission != admission
            || !self.original.same_original(original)
        {
            return Err(unavailable());
        }
        Ok(())
    }

    fn outcome(&self) -> Result<V2FounderRemoveOutcomeData<'_>, SyncError> {
        Ok(V2FounderRemoveOutcomeData {
            result: self
                .cell
                .as_ref()
                .and_then(PreparedTextCurrentRemove::completed_data)
                .ok_or_else(unavailable)?,
            unsigned: self.unsigned.as_ref().ok_or_else(unavailable)?,
        })
    }
}

impl V2FounderRemoveSigning<'_> {
    pub fn sign_header_native(
        &mut self,
        original: &NativeChatSourceObservation,
        cut: &mut NativeChatPreparedKeyCut,
        ends: &mut NativeChatKeyEnds,
    ) -> Result<(), SyncError> {
        self.negative.require_live_data()?;
        let pending = &mut *self.pending;
        if !pending.original.same_original(original) {
            return Err(unavailable());
        }
        if pending.raw.is_some() {
            return Ok(());
        }
        let unsigned = pending.unsigned.as_ref().ok_or_else(unavailable)?;
        let signer = self.seed.body.context.spec_data().signer;
        let device = &self.seed.body.device;
        if !device.retained_ed25519_key_matches(&signer) {
            return Err(unavailable());
        }
        let purpose = NativeChatKeyPurpose::RemovalHeader;
        cut.require_never_entered_data(original, purpose)
            .map_err(|_| unavailable())?;
        pending
            .header_entry
            .retry_local_refusal(cut, original, purpose)?;
        pending.header_entry.enter(cut, original, purpose, ends)?;
        pending.raw = Some(
            device
                .sign_retained_ed25519_bounded(
                    &signer,
                    unsigned.signing_bytes_data(),
                    MAX_HEADER_SEMANTIC_BYTES,
                )
                .map_err(|_| unavailable())?,
        );
        Ok(())
    }
}

impl PreparedV2FounderRemoveInstall<'_> {
    pub fn expected_data(&self) -> V2InstalledFounderAddData {
        self.after
    }

    pub fn check_host_selection_data(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        candidate: &V2GeneralCandidate,
    ) -> Result<(), SyncError> {
        self.negative.require_live_data()?;
        let replay = verifier.replay_data();
        let founder = self.member.admission_data().admission_id();
        let selected_founder = verifier.admission(founder).map_err(|_| unavailable())?;
        if replay.header().id() != self.parent.header
            || replay.header().spec().state != self.parent.state
            || replay.mls_epoch_data() != self.parent.epoch
            || !self.member.same_original(&selected_founder)
        {
            return Err(unavailable());
        }
        verifier
            .check_host_selection_data(&self.selection(), candidate)
            .map_err(|_| unavailable())
    }

    /// The owning host supplies the original covered Head before the exact
    /// Core and SDK moves. Return custody later outside the final writer.
    pub fn finish_host_selected(
        self,
        verifier: &mut V2GeneralPrefixVerifier,
        candidate: &mut Option<V2GeneralCandidate>,
    ) -> Result<(), SyncError> {
        self.check_host_selection_data(verifier, candidate.as_ref().ok_or_else(unavailable)?)?;
        let core = verifier
            .advance_host_selected(&self.selection(), candidate)
            .map_err(|_| unavailable())?;
        *self.retired = Some((core, self.mls.install()));
        *self.installed = Some(self.after);
        Ok(())
    }

    fn selection(&self) -> V2HostSelectionExpectation {
        V2HostSelectionExpectation::from_trusted_host(self.after.header, self.after.state)
    }
}

impl RetiredV2FounderRemoveInstall {
    pub fn take_frozen_data(&mut self) -> Option<V2FrozenAdmission> {
        self.original.retired.as_mut()?.0.take_frozen_data()
    }

    pub fn take_close_completion(&mut self) -> Option<V2AdmissionCloseCompletion> {
        self.original.retired.as_mut()?.0.take_close_completion()
    }
}
