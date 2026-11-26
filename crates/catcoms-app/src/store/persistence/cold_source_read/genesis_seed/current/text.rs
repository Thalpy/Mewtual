//! Sealed call-throughs to each actual family's original node.
//!
//! No callback lends the node/SDK/RNG and no extractor can adopt it. The operational host
//! supplies actual selected-source/index/signing admission separately. These methods check
//! cached original ownership; they deliberately never invoke strict genesis physical proof.
use super::UnexposedV2GenesisCurrent;

use catcoms_mls::persist::text_current_add::TextCurrentAddAdmission;
use catcoms_replication::chat::{
    owner_header::{OwnerBodyId, SignedOwnerHeader},
    V2GeneralCandidate, V2GeneralPrefixVerifier,
};
use catcoms_rt::{NativeChatKeyEnds, NativeChatPreparedKeyCut, NativeChatSourceObservation};
use catcoms_sync::owner_v2::{
    RetiredV2FounderRemoveInstall, V2FounderAddOutcomeData, V2FounderAddRequestData,
    V2FounderRemoveOutcomeData,
};

impl<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore> UnexposedV2GenesisCurrent<T, R> {
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn execute_governed_removal(
        &mut self,
        store: &crate::store::ServerStore,
        verifier: &V2GeneralPrefixVerifier,
        admission: OwnerBodyId,
        original: &NativeChatSourceObservation,
        (payment, cut, ends): (
            &mut Option<TextCurrentAddAdmission>,
            &mut NativeChatPreparedKeyCut,
            &mut NativeChatKeyEnds,
        ),
    ) -> Result<(), crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        let node = self.governed_text_node_mut().ok_or_else(refused)?;
        node.prepare_removal_data(verifier, admission, original.clone())?;
        node.execute_removal_native(verifier, admission, original, payment, cut, ends)?;
        self.require_text_original_data(store)
            .map_err(|_| refused())
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn governed_removal_outcome(
        &self,
        store: &crate::store::ServerStore,
    ) -> Result<V2FounderRemoveOutcomeData<'_>, crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        Ok(self
            .governed_text_node()
            .ok_or_else(refused)?
            .removal_outcome_data()?)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn materialize_governed_removal(
        &mut self,
        store: &crate::store::ServerStore,
    ) -> Result<SignedOwnerHeader, crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        Ok(self
            .governed_text_node_mut()
            .ok_or_else(refused)?
            .materialize_signed_removal_data()?
            .clone())
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn derive_governed_removal_selection(
        &self,
        store: &crate::store::ServerStore,
        verifier: &mut V2GeneralPrefixVerifier,
    ) -> Result<V2GeneralCandidate, crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        Ok(self
            .governed_text_node()
            .ok_or_else(refused)?
            .derive_removal_selection_data(verifier)?)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn take_retired_governed_removal(
        &mut self,
    ) -> Result<RetiredV2FounderRemoveInstall, crate::AppError> {
        use crate::store::critical_namespaces::refused;
        Ok(self
            .governed_text_node_mut()
            .ok_or_else(refused)?
            .take_retired_removal_data()?)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn select_governed_dependency_access(
        &self,
        store: &crate::store::ServerStore,
        verifier: &catcoms_replication::chat::V2GeneralPrefixVerifier,
        channel: u128,
    ) -> Result<catcoms_sync::owner_v2::V2FounderMemberDependencyAccess<'_>, crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        Ok(self
            .governed_text_node()
            .ok_or_else(refused)?
            .select_member_dependency_access(verifier, channel)?)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn prepare_governed_member_subscription(
        &self,
        store: &crate::store::ServerStore,
        verifier: &catcoms_replication::chat::V2GeneralPrefixVerifier,
        channel: u128,
    ) -> Result<
        (
            catcoms_rt::Topic,
            catcoms_sync::owner_v2::V2MemberPublicationObservation,
        ),
        crate::AppError,
    > {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        Ok(self
            .governed_text_node()
            .ok_or_else(refused)?
            .prepare_member_subscription(verifier, channel)?)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn open_governed_member_frame(
        &self,
        store: &crate::store::ServerStore,
        verifier: &catcoms_replication::chat::V2GeneralPrefixVerifier,
        channel: u128,
        topic: &catcoms_rt::Topic,
        wire: &[u8],
    ) -> Result<catcoms_sync::owner_v2::SelectedV2MemberEnvelope, crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        Ok(self
            .governed_text_node()
            .ok_or_else(refused)?
            .open_selected_member_envelope(verifier, channel, topic, wire)?)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn prepare_governed_member_submission(
        &mut self,
        store: &crate::store::ServerStore,
        verifier: &catcoms_replication::chat::V2GeneralPrefixVerifier,
        item: &crate::store::persistence::v2_member_outbox::MemberDebtItem,
    ) -> Result<catcoms_sync::owner_v2::PreparedV2MemberSubmission, crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        let node = self.governed_text_node_mut().ok_or_else(refused)?;
        Ok(node.prepare_member_submission(
            verifier,
            item.action().spec().realm.channel(),
            item.admission(),
            item.original_bytes(),
        )?)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn derive_governed_core_selection(
        &self,
        store: &crate::store::ServerStore,
        verifier: &mut catcoms_replication::chat::V2GeneralPrefixVerifier,
    ) -> Result<catcoms_replication::chat::V2GeneralCandidate, crate::AppError> {
        use crate::store::critical_namespaces::refused;
        use catcoms_replication::chat::owner_state::transition::ReplayBody;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        let node = self.governed_text_node().ok_or_else(refused)?;
        let (body, unsigned) = node.unsigned_admission_data()?;
        verifier
            .derive_successor(
                node.signed_admission_data()?,
                ReplayBody::Admission(body),
                unsigned.image_data(),
            )
            .map_err(|_| refused())
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn take_retired_governed_install(
        &mut self,
    ) -> Result<catcoms_sync::owner_v2::RetiredV2FounderAddInstall, crate::AppError> {
        Ok(self
            .governed_text_node_mut()
            .ok_or_else(crate::store::critical_namespaces::refused)?
            .take_retired_install_data()?)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn materialize_governed_response(
        &mut self,
        store: &crate::store::ServerStore,
    ) -> Result<(), crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        self.governed_text_node_mut()
            .ok_or_else(refused)?
            .materialize_welcome_data()?;
        self.require_text_original_data(store)
            .map_err(|_| refused())
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn governed_response_data(
        &self,
        store: &crate::store::ServerStore,
    ) -> Result<catcoms_sync::V2JoinResponseData<'_>, crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        Ok(self
            .governed_text_node()
            .ok_or_else(refused)?
            .join_response_data()?)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn cached_governed_welcome_raw(
        &self,
        store: &crate::store::ServerStore,
    ) -> Result<Option<[u8; 64]>, crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        let node = self.governed_text_node().ok_or_else(refused)?;
        Ok(node.raw_welcome_signature_data().ok().copied())
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn freeze_governed_welcome(
        &mut self,
        store: &crate::store::ServerStore,
    ) -> Result<Vec<u8>, crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        let node = self.governed_text_node_mut().ok_or_else(refused)?;
        node.prepare_welcome_data()?;
        let preimage = node.unsigned_welcome_data()?.1.to_vec();
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        Ok(preimage)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn materialize_governed_header(
        &mut self,
        store: &crate::store::ServerStore,
    ) -> Result<catcoms_replication::chat::owner_header::SignedOwnerHeader, crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        let node = self.governed_text_node_mut().ok_or_else(refused)?;
        node.materialize_admission_data()?;
        let header = node.signed_admission_data()?.clone();
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        Ok(header)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn cached_governed_header_raw(
        &self,
        store: &crate::store::ServerStore,
    ) -> Result<Option<[u8; 64]>, crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        let node = self.governed_text_node().ok_or_else(refused)?;
        Ok(node.raw_admission_signature_data().ok().copied())
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn prepare_governed_sdk(
        &mut self,
        store: &crate::store::ServerStore,
        request: V2FounderAddRequestData<'_>,
        original: NativeChatSourceObservation,
        admission: &mut Option<TextCurrentAddAdmission>,
        [sdk, header, welcome]: [NativeChatPreparedKeyCut; 3],
    ) -> Result<(), crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        let node = self.governed_text_node_mut().ok_or_else(refused)?;
        if let Some(payment) = admission.take() {
            if let Err(refusal) =
                node.prepare_add_data(request, original, payment, sdk, header, welcome)
            {
                let (_, payment, _, _, _) = *refusal;
                *admission = Some(payment);
                return Err(refused());
            }
        } else {
            // Only the same original never-entered/refused SDK stage can use
            // a replacement cut. Existing live custody stays with the node.
            use catcoms_sync::owner_v2::V2FounderAddNativeStage as Stage;
            for (stage, cut) in [
                (Stage::Sdk, sdk),
                (Stage::GoverningHeader, header),
                (Stage::Welcome, welcome),
            ] {
                drop(node.prepare_add_native_stage_data(stage, cut));
            }
        }
        self.require_text_original_data(store)
            .map_err(|_| refused())
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn run_governed_sdk(
        &mut self,
        store: &crate::store::ServerStore,
        request: V2FounderAddRequestData<'_>,
        replay: &catcoms_replication::chat::owner_state::transition::ReplayMath,
    ) -> Result<(), crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        let node = self.governed_text_node_mut().ok_or_else(refused)?;
        if node.completed_add_data().is_err() {
            node.execute_add_data_native(request)?;
        }
        node.prepare_admission_data(replay)?;
        node.outcome_data()?.require_request_data(request)?;
        self.require_text_original_data(store)
            .map_err(|_| refused())
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn preflight_add_package_data(
        &self,
        store: &crate::store::ServerStore,
        request: catcoms_sync::owner_v2::V2FounderAddRequestData<'_>,
        limits: catcoms_mls::persist::text_current_add::TextCurrentAddLimits,
        selected: Option<&catcoms_replication::chat::owner_state::transition::ReplayMath>,
    ) -> Result<(), crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        match (
            self.genesis_text_node(),
            self.governed_text_node(),
            selected,
        ) {
            (Some(node), None, None) => node.preflight_first_add_package_data(request, limits)?,
            (None, Some(node), None) => node.preflight_first_add_package_data(request, limits)?,
            (None, Some(node), Some(replay)) => {
                node.preflight_subsequent_add_package_data(replay, request, limits)?
            }
            _ => return Err(refused()),
        }
        self.require_text_original_data(store)
            .map_err(|_| refused())
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn governed_fetch_verifier(
        &self,
        store: &crate::store::ServerStore,
        verifier: &catcoms_replication::chat::V2GeneralPrefixVerifier,
    ) -> Result<[u8; 32], crate::AppError> {
        use crate::store::critical_namespaces::refused;
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        let request = self
            .governed_text_node()
            .ok_or_else(refused)?
            .member_add_request_data(verifier)?;
        Ok(request.verifier_data())
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn retire_add_for_closed_ui(
        &mut self,
        closed: &[Option<&catcoms_rt::NativeChatKeyDrain>],
    ) -> Result<(), crate::AppError> {
        if let Some(node) = self.governed_text_node_mut() {
            drop(node.retire_add_for_closed_ui(closed)?);
        }
        Ok(())
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn discard_add_for_closed_source(
        &mut self,
        closed: &catcoms_rt::NativeChatKeyDrain,
    ) -> Result<(), crate::AppError> {
        if let Some(node) = self.governed_text_node_mut() {
            node.discard_add_for_closed_source(closed)?;
        }
        Ok(())
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn governed_outcome_data(
        &self,
        store: &crate::store::ServerStore,
    ) -> Result<V2FounderAddOutcomeData<'_>, crate::AppError> {
        self.require_text_original_data(store)
            .map_err(|_| crate::store::critical_namespaces::refused())?;
        self.governed_text_node()
            .ok_or_else(crate::store::critical_namespaces::refused)?
            .outcome_data()
            .map_err(crate::AppError::from)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn prepare_governed_text_node(
        &mut self,
        store: &crate::store::ServerStore,
    ) -> Result<(), crate::AppError> {
        use crate::store::critical_namespaces::refused;
        use catcoms_sync::owner_v2::{UnboundV2GenesisNode, FOUNDER_GOVERNANCE_DATA_BYTES};
        self.require_text_original_data(store)
            .map_err(|_| refused())?;
        if self.governed.is_some() {
            return Ok(());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let bytes = FOUNDER_GOVERNANCE_DATA_BYTES
            .checked_add(4 * std::mem::size_of::<super::GovernedWarmNode<T, R>>())
            .ok_or_else(refused)?;
        let fee = pool.try_reserve(bytes)?;
        fee.validate(pool, bytes)?;
        let mut governed = Box::new(super::GovernedWarmNode {
            node: None,
            _fee: fee,
        });
        UnboundV2GenesisNode::prepare_governance_into_data(&mut self.node, &mut governed.node)?;
        self.governed = Some(governed);
        self.require_text_original_data(store)
            .map_err(|_| refused())
    }

    /// Warm-only original, never a node extracted from decoded cold metadata.
    /// No raw SDK/RNG loan escapes; the SAME node owns every retained draw.
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn prepare_text_invite_data(
        &mut self,
        store: &crate::store::ServerStore,
        request: catcoms_sync::owner_v2::V2FounderInviteRequestData,
        native: &catcoms_rt::NativeChatSourceObservation,
        slot: &mut Option<catcoms_sync::owner_v2::PreparedV2FounderInviteData>,
        selected: Option<&catcoms_replication::chat::V2GeneralPrefixVerifier>,
    ) -> Result<(), crate::AppError> {
        self.require_text_original_data(store)
            .map_err(|_| crate::store::critical_namespaces::refused())?;
        let result = if let Some(verifier) = selected {
            self.governed_text_node_mut()
                .ok_or_else(crate::store::critical_namespaces::refused)?
                .prepare_selected_invite_data(verifier, request, native, slot)
        } else {
            self.genesis_text_node_mut()
                .ok_or_else(crate::store::critical_namespaces::refused)?
                .prepare_invite_data(request, native, slot)
        }
        .map_err(crate::AppError::from);
        self.require_text_original_data(store)
            .map_err(|_| crate::store::critical_namespaces::refused())?;
        result
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn preflight_text_invite_signing(
        &self,
        store: &crate::store::ServerStore,
        original: &catcoms_sync::owner_v2::PreparedV2FounderInviteData,
        selected: Option<&catcoms_replication::chat::V2GeneralPrefixVerifier>,
    ) -> Result<(), crate::AppError> {
        self.require_text_original_data(store)
            .map_err(|_| crate::store::critical_namespaces::refused())?;
        if let Some(verifier) = selected {
            self.governed_text_node()
                .ok_or_else(crate::store::critical_namespaces::refused)?
                .preflight_selected_invite_signing(verifier, original)
        } else {
            self.genesis_text_node()
                .ok_or_else(crate::store::critical_namespaces::refused)?
                .preflight_invite_signing_data(original)
        }
        .map_err(crate::AppError::from)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn sign_text_invite(
        &self,
        store: &crate::store::ServerStore,
        original: &mut catcoms_sync::owner_v2::PreparedV2FounderInviteData,
        cut: &mut catcoms_rt::NativeChatPreparedKeyCut,
        ends: &mut catcoms_rt::NativeChatKeyEnds,
        selected: Option<&catcoms_replication::chat::V2GeneralPrefixVerifier>,
    ) -> Result<(), crate::AppError> {
        self.require_text_original_data(store)
            .map_err(|_| crate::store::critical_namespaces::refused())?;
        if let Some(verifier) = selected {
            self.governed_text_node()
                .ok_or_else(crate::store::critical_namespaces::refused)?
                .sign_selected_invite_native(verifier, original, cut, ends)
        } else {
            self.genesis_text_node()
                .ok_or_else(crate::store::critical_namespaces::refused)?
                .sign_invite_native_data(original, cut, ends)
        }
        .map_err(crate::AppError::from)
    }
}

macro_rules! node_call_throughs {
    (@genesis $this:ident, warm) => {$this.node.as_ref()};
    (@genesis $this:ident) => {Some(&$this.node)};
    (@genesis_mut $this:ident, warm) => {$this.node.as_mut()};
    (@genesis_mut $this:ident) => {Some(&mut $this.node)};
    (@governed $this:ident, warm) => {$this.governed.as_ref().and_then(|owner| owner.node.as_ref())};
    (@governed $this:ident) => {None};
    (@governed_mut $this:ident, warm) => {$this.governed.as_mut().and_then(|owner| owner.node.as_mut())};
    (@governed_mut $this:ident) => {None};
    ($ty:ident $(, $mode:ident)?) => {
        impl<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore> $ty<T, R> {
            fn genesis_text_node(&self) -> Option<&catcoms_sync::owner_v2::UnboundV2GenesisNode<T, R>> {
                node_call_throughs!(@genesis self $(, $mode)?)
            }
            fn genesis_text_node_mut(
                &mut self,
            ) -> Option<&mut catcoms_sync::owner_v2::UnboundV2GenesisNode<T, R>> {
                node_call_throughs!(@genesis_mut self $(, $mode)?)
            }
            fn governed_text_node(
                &self,
            ) -> Option<&catcoms_sync::owner_v2::PreparedV2FounderGovernanceNode<T, R>> {
                node_call_throughs!(@governed self $(, $mode)?)
            }
            fn governed_text_node_mut(
                &mut self,
            ) -> Option<&mut catcoms_sync::owner_v2::PreparedV2FounderGovernanceNode<T, R>> {
                node_call_throughs!(@governed_mut self $(, $mode)?)
            }

            pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn require_text_original_data(
                &self,
                store: &crate::store::ServerStore,
            ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
                use catcoms_sync::owner_v2::V2CreateRefusal;
                let owner = self.text_owner_ref().map_err(|_| V2CreateRefusal::Source)?;
                let writer = owner.observe_writer();
                owner.require_original_negative(store, &writer)
                    .map_err(|_| V2CreateRefusal::Source)
            }

            /// Outside every global writer. The SourceRef created by the first check
            /// ends when that helper RETURNS, before mutating this SAME node's RNG.
            /// The caller's retained slot owns partial/materialized DATA even on Err.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn prepare_text_create_data(
                &mut self,
                store: &crate::store::ServerStore,
                input: catcoms_sync::owner_v2::V2CreateRequest<'_>,
                slot: &mut Option<catcoms_sync::owner_v2::PreparedV2CreateData>,
            ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
                self.require_text_original_data(store)?;
                let node = self
                    .genesis_text_node_mut()
                    .ok_or(catcoms_sync::owner_v2::V2CreateRefusal::Source)?;
                let result = node.prepare_create_data(input, slot);
                self.require_text_original_data(store)?;
                result
            }

            /// Outside global writers; full parent validation is intentionally not the
            /// short key-use cut. Actual model/payment remains in Store pending custody.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn prepare_text_append_data(
                &self,
                store: &crate::store::ServerStore,
                model: &catcoms_replication::chat::owner_state::transition::ReplayMath,
                action: &catcoms_replication::chat::SignedAction,
                slot: &mut Option<catcoms_sync::owner_v2::PreparedV2AppendData>,
            ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
                self.require_text_original_data(store)?;
                let node = self
                    .genesis_text_node()
                    .ok_or(catcoms_sync::owner_v2::V2CreateRefusal::Source)?;
                let result = node.prepare_create_append_data(model, action, slot);
                self.require_text_original_data(store)?;
                result
            }

            /// Full preparation/validation before the global-writer signing admission.
            /// The observation contains the actual original opaque plan, not caller bytes.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn observe_text_append_plan(
                &self,
                store: &crate::store::ServerStore,
                model: &catcoms_replication::chat::owner_state::transition::ReplayMath,
                prepared: &catcoms_sync::owner_v2::PreparedV2AppendData,
            ) -> Result<catcoms_sync::owner_v2::V2AppendPlanObservation,
                        catcoms_sync::owner_v2::V2CreateRefusal> {
                self.require_text_original_data(store)?;
                let node = self
                    .genesis_text_node()
                    .ok_or(catcoms_sync::owner_v2::V2CreateRefusal::Source)?;
                let result = prepared.observe_plan_data(node, model);
                self.require_text_original_data(store)?;
                result
            }

            /// Root's actual selected-I/current/close guard wraps this family-only call.
            /// The node compares SAME opaque plan and stores raw64 before late negatives;
            /// no Header verification, derivation, encoding or RNG is performed here.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn sign_text_append_bytes(
                &self,
                store: &crate::store::ServerStore,
                original_plan: &catcoms_sync::owner_v2::V2AppendPlanObservation,
                prepared: &mut catcoms_sync::owner_v2::PreparedV2AppendData,
            ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
                self.require_text_original_data(store)?;
                let node = self
                    .genesis_text_node()
                    .ok_or(catcoms_sync::owner_v2::V2CreateRefusal::Source)?;
                let result = node.sign_reserved_append_bytes_data(original_plan, prepared);
                self.require_text_original_data(store)?;
                result
            }

            /// Outside global writers. Reconstruct and verify Header from existing raw;
            /// the paired Core verifier derives the candidate exactly ONCE afterwards.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn materialize_text_append(
                &self,
                store: &crate::store::ServerStore,
                model: &catcoms_replication::chat::owner_state::transition::ReplayMath,
                prepared: &mut catcoms_sync::owner_v2::PreparedV2AppendData,
            ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
                self.require_text_original_data(store)?;
                let node = self
                    .genesis_text_node()
                    .ok_or(catcoms_sync::owner_v2::V2CreateRefusal::Source)?;
                let result = node.materialize_signed_append_data(model, prepared);
                self.require_text_original_data(store)?;
                result
            }

            /// Borrow only bounded unsigned DATA for the actual private I candidate.
            /// No node/SDK/Clock/RNG or signing callback crosses this boundary.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn borrow_unsigned_text_append<'a>(
                &'a self,
                store: &crate::store::ServerStore,
                model: &'a catcoms_replication::chat::owner_state::transition::ReplayMath,
                prepared: &'a catcoms_sync::owner_v2::PreparedV2AppendData,
            ) -> Result<catcoms_sync::owner_v2::V2UnsignedAppendDataView<'a>,
                        catcoms_sync::owner_v2::V2CreateRefusal> {
                self.require_text_original_data(store)?;
                let node = self
                    .genesis_text_node()
                    .ok_or(catcoms_sync::owner_v2::V2CreateRefusal::Source)?;
                prepared.borrow_unsigned_data(node, model)
            }

            /// ORDINARY-member seams on the SAME original node. None reaches the founder
            /// Append/Header lane. Pre-admission: the full retained device key.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn text_ordinary_member_signer(
                &self,
                store: &crate::store::ServerStore,
            ) -> Result<[u8; 32], catcoms_sync::owner_v2::V2MemberSignRefusal> {
                use catcoms_sync::owner_v2::V2MemberSignRefusal;
                self.require_text_original_data(store)
                    .map_err(|_| V2MemberSignRefusal::Source)?;
                if let Some(node) = self.governed_text_node() {
                    return node.ordinary_member_signer_data();
                }
                self.genesis_text_node()
                    .ok_or(V2MemberSignRefusal::Source)?
                    .ordinary_member_signer_data()
            }

            /// Outside every writer: the request's one Clock/RNG draw through this node's
            /// retained seams. The caller's slot caches each value before late checks.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn draw_text_ordinary_inputs(
                &mut self,
                store: &crate::store::ServerStore,
                slot: &mut catcoms_sync::owner_v2::V2MemberDraftInputs,
            ) -> Result<(u64, [u8; 16]), catcoms_sync::owner_v2::V2MemberSignRefusal> {
                use catcoms_sync::owner_v2::V2MemberSignRefusal;
                self.require_text_original_data(store)
                    .map_err(|_| V2MemberSignRefusal::Source)?;
                let result = if let Some(node) = self.governed_text_node_mut() {
                    node.draw_ordinary_draft_inputs(slot)
                } else {
                    self.genesis_text_node_mut()
                        .ok_or(V2MemberSignRefusal::Source)?
                        .draw_ordinary_draft_inputs(slot)
                };
                self.require_text_original_data(store)
                    .map_err(|_| V2MemberSignRefusal::Source)?;
                result
            }

            /// Inside the host's Current/group guard, with Core's original reader held by
            /// `guard`. The caller ran the full original-owner negatives (via
            /// `text_ordinary_member_signer`) inside this SAME Current callback just before
            /// Core admission, so nothing but the node's own key cut runs here. raw64 is
            /// cached in the Plan by the node itself, so no late check can lose it. `inputs` is
            /// the request's draw cell: the node refuses a request drawn on another seed.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn sign_text_ordinary_admitted<'a>(
                &self,
                inputs: &catcoms_sync::owner_v2::V2MemberDraftInputs,
                guard: catcoms_replication::chat::V2OrdinarySigningGuard<'a>,
            ) -> Result<
                catcoms_replication::chat::V2OrdinarySigningGuard<'a>,
                catcoms_sync::owner_v2::V2MemberSignRefusal,
            > {
                self.genesis_text_node()
                    .ok_or(catcoms_sync::owner_v2::V2MemberSignRefusal::Source)?
                    .sign_ordinary_admitted(inputs, guard)
            }

            /// Concrete native edge on the SAME family/node; caller retains
            /// the native pins and Ends outside Current/Core guards.
            pub(in crate::store::persistence::cold_source_read::genesis_seed::current)
            fn sign_text_ordinary_admitted_native<'a>(
                &self,
                inputs: &catcoms_sync::owner_v2::V2MemberDraftInputs,
                guard: catcoms_replication::chat::V2OrdinarySigningGuard<'a>,
                original: &catcoms_rt::NativeChatSourceObservation,
                cut: &mut catcoms_rt::NativeChatPreparedKeyCut,
                ends: &mut catcoms_rt::NativeChatKeyEnds,
            ) -> Result<catcoms_replication::chat::V2OrdinarySigningGuard<'a>,
                catcoms_sync::owner_v2::V2MemberSignRefusal> {
                if let Some(node) = self.governed_text_node() {
                    return node.sign_ordinary_admitted_native(inputs, guard, original, cut, ends);
                }
                self.genesis_text_node()
                    .ok_or(catcoms_sync::owner_v2::V2MemberSignRefusal::Source)?
                    .sign_ordinary_admitted_native(inputs, guard, original, cut, ends)
            }
        }
    };
}

pub(super) use node_call_throughs;
node_call_throughs!(UnexposedV2GenesisCurrent, warm);
