//! Closed ownership adapters for the actual operational host.
//!
//! The original wrappers have meaningful Drop implementations. Keep them whole instead of
//! extracting a node, reconstructing an SDK, or cancelling an old owner during conversion.
//! This module performs cached association checks only. Operational physical selection is
//! proved separately under the new Scope/canonical/Root guards.

use crate::store::chat_admission::v2_current::{
    ColdV2CurrentPreparation, ColdV2InitializingPreparation, V2CurrentPreparation,
};
use crate::store::critical_namespaces::archive_current_codec::V2GroupPinId;
use crate::store::critical_namespaces::current_layout::PreparedV2GenesisGroupNamespace;
use crate::store::critical_namespaces::current_layout::V2CurrentLayoutStore;
use crate::store::critical_namespaces::refused;
use crate::store::owner_evidence::canonical::v2_registration::V2WriterObservation;
use crate::store::persistence::{
    GenesisCurrentOwnerRef, GenesisSourceDataRef, UnexposedColdV2GenesisCurrent,
    UnexposedRepairedV2GenesisCurrent, UnexposedV2GenesisCurrent,
};
use crate::store::ServerStore;
use crate::AppError;

/// No decoded role or caller-selected constructor can create this core. Each variant owns
/// the SAME source/node/T/R/Clock/Scope/writer and its existing independent resource fees.
enum OwnedCurrent<T, R> {
    Warm(UnexposedV2GenesisCurrent<T, R>),
    Cold(UnexposedColdV2GenesisCurrent<T, R>),
    Repaired(UnexposedRepairedV2GenesisCurrent<T, R>),
}

/// Inert owning transfer only. The host must separately fund its actual enum/phase layouts
/// before any new Box, observation, selected-head capture or operational filesystem effect.
/// There is intentionally no into-node/Server/SDK/parts accessor and no adopting trait.
pub(in crate::store) struct V2TextOwnedCurrent<T, R> {
    core: OwnedCurrent<T, R>,
}

/// A sealed loan of actual original associations. The inner Genesis name describes the
/// retained original source/Scope family, NOT the physical validator used for later heads.
/// In particular no method here reads activation.seed or accepts a genesis-only inventory.
pub(in crate::store) struct V2TextOwnerRef<'owner> {
    original: GenesisCurrentOwnerRef<'owner>,
}

impl<T, R> V2TextOwnedCurrent<T, R> {
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn from_warm(
        current: UnexposedV2GenesisCurrent<T, R>,
    ) -> Self {
        Self {
            core: OwnedCurrent::Warm(current),
        }
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn from_cold(
        current: UnexposedColdV2GenesisCurrent<T, R>,
    ) -> Self {
        Self {
            core: OwnedCurrent::Cold(current),
        }
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn from_repaired(
        current: UnexposedRepairedV2GenesisCurrent<T, R>,
    ) -> Self {
        Self {
            core: OwnedCurrent::Repaired(current),
        }
    }

    /// Cached settled prerequisite plus exact original owner association. The family
    /// call-throughs inspect actual saved cleanup (or the independent absent ColdHead),
    /// not missing-file guesses. A lost cleanup reply must be retried by that same wrapper.
    pub(in crate::store) fn borrow_current(&self) -> Result<V2TextOwnerRef<'_>, AppError> {
        let original = match &self.core {
            OwnedCurrent::Warm(current) => current.text_owner_ref()?,
            OwnedCurrent::Cold(current) => current.text_owner_ref()?,
            OwnedCurrent::Repaired(current) => current.text_owner_ref()?,
        };
        Ok(V2TextOwnerRef { original })
    }

    /// Root invokes this only in its actual Original/settlement phase, before promotion.
    /// Cold dispatches its genuine Present/Absent selection, never an error fallback.
    pub(super) fn retry_seed_settlement(
        &mut self,
        layout: &V2CurrentLayoutStore,
        rng: &mut impl catcoms_rt::CryptoRngCore,
    ) -> Result<(), AppError> {
        match &mut self.core {
            OwnedCurrent::Warm(current) => current.retry_seed_settlement(layout, rng),
            OwnedCurrent::Cold(current) => current.retry_text_seed_settlement(layout, rng),
            OwnedCurrent::Repaired(current) => current.retry_seed_settlement(layout, rng),
        }
    }

    /// Negative revocation only. Root closes projection/commit admission before invoking
    /// this, and joins running work separately; no Drop blocks waiting for a worker.
    pub(super) fn cancel_original(&self) {
        match &self.core {
            OwnedCurrent::Warm(current) => current.cancel_for_text(),
            OwnedCurrent::Cold(current) => current.cancel_for_text(),
            OwnedCurrent::Repaired(current) => current.cancel_for_text(),
        }
    }

    /// Actual negative control for the genuine warm-origin regression. It exposes no
    /// writer/source/Runtime and cannot construct a positive observation or registration.
    #[cfg(all(test, windows))]
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn cancel_warm_writer_for_test(
        &self,
    ) {
        match &self.core {
            OwnedCurrent::Warm(current) => current.writer.cancel_original(),
            _ => panic!("test helper requires the actual warm fixture family"),
        }
    }
}

impl<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore> V2TextOwnedCurrent<T, R> {
    pub(super) fn prepare_governed_member_submission(
        &mut self,
        store: &ServerStore,
        verifier: &catcoms_replication::chat::V2GeneralPrefixVerifier,
        item: &crate::store::persistence::v2_member_outbox::MemberDebtItem,
    ) -> Result<catcoms_sync::owner_v2::PreparedV2MemberSubmission, AppError> {
        match &mut self.core {
            OwnedCurrent::Warm(current) => {
                current.prepare_governed_member_submission(store, verifier, item)
            }
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn derive_governed_core_selection(
        &self,
        store: &ServerStore,
        verifier: &mut catcoms_replication::chat::V2GeneralPrefixVerifier,
    ) -> Result<catcoms_replication::chat::V2GeneralCandidate, AppError> {
        match &self.core {
            OwnedCurrent::Warm(current) => current.derive_governed_core_selection(store, verifier),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn take_retired_governed_install(
        &mut self,
    ) -> Result<catcoms_sync::owner_v2::RetiredV2FounderAddInstall, AppError> {
        match &mut self.core {
            OwnedCurrent::Warm(current) => current.take_retired_governed_install(),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn borrow_governed_install_parts(
        &mut self,
        store: &ServerStore,
    ) -> Result<
        (
            V2TextOwnerRef<'_>,
            catcoms_sync::owner_v2::PreparedV2FounderAddInstall<'_>,
        ),
        AppError,
    > {
        match &mut self.core {
            OwnedCurrent::Warm(current) => {
                let (original, install) = current.borrow_governed_install_parts(store)?;
                Ok((V2TextOwnerRef { original }, install))
            }
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn materialize_governed_response(
        &mut self,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        match &mut self.core {
            OwnedCurrent::Warm(current) => current.materialize_governed_response(store),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn governed_response_data(
        &self,
        store: &ServerStore,
    ) -> Result<catcoms_sync::V2JoinResponseData<'_>, AppError> {
        match &self.core {
            OwnedCurrent::Warm(current) => current.governed_response_data(store),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn cached_governed_welcome_raw(
        &self,
        store: &ServerStore,
    ) -> Result<Option<[u8; 64]>, AppError> {
        match &self.core {
            OwnedCurrent::Warm(current) => current.cached_governed_welcome_raw(store),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn freeze_governed_welcome(
        &mut self,
        store: &ServerStore,
    ) -> Result<Vec<u8>, AppError> {
        match &mut self.core {
            OwnedCurrent::Warm(current) => current.freeze_governed_welcome(store),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn materialize_governed_header(
        &mut self,
        store: &ServerStore,
    ) -> Result<catcoms_replication::chat::owner_header::SignedOwnerHeader, AppError> {
        match &mut self.core {
            OwnedCurrent::Warm(current) => current.materialize_governed_header(store),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn cached_governed_header_raw(
        &self,
        store: &ServerStore,
    ) -> Result<Option<[u8; 64]>, AppError> {
        match &self.core {
            OwnedCurrent::Warm(current) => current.cached_governed_header_raw(store),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn borrow_governed_signing_parts(
        &mut self,
    ) -> Result<
        (
            V2TextOwnerRef<'_>,
            catcoms_sync::owner_v2::V2FounderAddSigning<'_>,
        ),
        AppError,
    > {
        match &mut self.core {
            OwnedCurrent::Warm(current) => {
                let (original, signing) = current.borrow_governed_signing_parts()?;
                Ok((V2TextOwnerRef { original }, signing))
            }
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn prepare_governed_sdk(
        &mut self,
        store: &ServerStore,
        request: catcoms_sync::owner_v2::V2FounderAddRequestData<'_>,
        original: catcoms_rt::NativeChatSourceObservation,
        admission: &mut Option<catcoms_mls::persist::text_current_add::TextCurrentAddAdmission>,
        cuts: [catcoms_rt::NativeChatPreparedKeyCut; 3],
    ) -> Result<(), AppError> {
        match &mut self.core {
            OwnedCurrent::Warm(current) => {
                current.prepare_governed_sdk(store, request, original, admission, cuts)
            }
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn run_governed_sdk(
        &mut self,
        store: &ServerStore,
        request: catcoms_sync::owner_v2::V2FounderAddRequestData<'_>,
        replay: &catcoms_replication::chat::owner_state::transition::ReplayMath,
    ) -> Result<(), AppError> {
        match &mut self.core {
            OwnedCurrent::Warm(current) => current.run_governed_sdk(store, request, replay),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn preflight_first_add_package_data(
        &self,
        store: &ServerStore,
        request: catcoms_sync::owner_v2::V2FounderAddRequestData<'_>,
        limits: catcoms_mls::persist::text_current_add::TextCurrentAddLimits,
    ) -> Result<(), AppError> {
        match &self.core {
            OwnedCurrent::Warm(current) => {
                current.preflight_first_add_package_data(store, request, limits)
            }
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn retire_add_for_closed_ui(
        &mut self,
        closed: &[Option<&catcoms_rt::NativeChatKeyDrain>],
    ) -> Result<(), AppError> {
        match &mut self.core {
            OwnedCurrent::Warm(current) => current.retire_add_for_closed_ui(closed),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Ok(()),
        }
    }

    pub(super) fn discard_add_for_closed_source(
        &mut self,
        closed: &catcoms_rt::NativeChatKeyDrain,
    ) -> Result<(), AppError> {
        match &mut self.core {
            OwnedCurrent::Warm(current) => current.discard_add_for_closed_source(closed),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Ok(()),
        }
    }

    pub(super) fn governed_outcome_data(
        &self,
        store: &ServerStore,
    ) -> Result<catcoms_sync::owner_v2::V2FounderAddOutcomeData<'_>, AppError> {
        match &self.core {
            OwnedCurrent::Warm(current) => current.governed_outcome_data(store),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn prepare_governed_node(&mut self, store: &ServerStore) -> Result<(), AppError> {
        match &mut self.core {
            OwnedCurrent::Warm(current) => current.prepare_governed_text_node(store),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    /// Only the real warm whole owner can prepare an invite. Cold/Repaired
    /// metadata never supplies a new signer or starts another original draw.
    pub(super) fn prepare_invite_data(
        &mut self,
        store: &ServerStore,
        request: catcoms_sync::owner_v2::V2FounderInviteRequestData,
        native: &catcoms_rt::NativeChatSourceObservation,
        slot: &mut Option<catcoms_sync::owner_v2::PreparedV2FounderInviteData>,
    ) -> Result<(), AppError> {
        match &mut self.core {
            OwnedCurrent::Warm(current) => {
                current.prepare_text_invite_data(store, request, native, slot)
            }
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn preflight_invite_signing(
        &self,
        store: &ServerStore,
        original: &catcoms_sync::owner_v2::PreparedV2FounderInviteData,
    ) -> Result<(), AppError> {
        match &self.core {
            OwnedCurrent::Warm(current) => current.preflight_text_invite_signing(store, original),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }

    pub(super) fn sign_invite(
        &self,
        store: &ServerStore,
        original: &mut catcoms_sync::owner_v2::PreparedV2FounderInviteData,
        cut: &mut catcoms_rt::NativeChatPreparedKeyCut,
        ends: &mut catcoms_rt::NativeChatKeyEnds,
    ) -> Result<(), AppError> {
        match &self.core {
            OwnedCurrent::Warm(current) => current.sign_text_invite(store, original, cut, ends),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }
    /// Root already admitted the exact selected request and separately paid producer
    /// buffers. Family helpers end their SourceRef borrow before the mutable RNG call.
    pub(super) fn prepare_create_data(
        &mut self,
        store: &ServerStore,
        input: catcoms_sync::owner_v2::V2CreateRequest<'_>,
        slot: &mut Option<catcoms_sync::owner_v2::PreparedV2CreateData>,
    ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
        match &mut self.core {
            OwnedCurrent::Warm(c) => c.prepare_text_create_data(store, input, slot),
            OwnedCurrent::Cold(c) => c.prepare_text_create_data(store, input, slot),
            OwnedCurrent::Repaired(c) => c.prepare_text_create_data(store, input, slot),
        }
    }
    pub(super) fn prepare_create_append_data(
        &self,
        store: &ServerStore,
        model: &catcoms_replication::chat::owner_state::transition::ReplayMath,
        action: &catcoms_replication::chat::SignedAction,
        slot: &mut Option<catcoms_sync::owner_v2::PreparedV2AppendData>,
    ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
        match &self.core {
            OwnedCurrent::Warm(c) => c.prepare_text_append_data(store, model, action, slot),
            OwnedCurrent::Cold(c) => c.prepare_text_append_data(store, model, action, slot),
            OwnedCurrent::Repaired(c) => c.prepare_text_append_data(store, model, action, slot),
        }
    }
    pub(super) fn observe_append_plan(
        &self,
        store: &ServerStore,
        model: &catcoms_replication::chat::owner_state::transition::ReplayMath,
        prepared: &catcoms_sync::owner_v2::PreparedV2AppendData,
    ) -> Result<
        catcoms_sync::owner_v2::V2AppendPlanObservation,
        catcoms_sync::owner_v2::V2CreateRefusal,
    > {
        match &self.core {
            OwnedCurrent::Warm(c) => c.observe_text_append_plan(store, model, prepared),
            OwnedCurrent::Cold(c) => c.observe_text_append_plan(store, model, prepared),
            OwnedCurrent::Repaired(c) => c.observe_text_append_plan(store, model, prepared),
        }
    }
    /// Only the operational parent can call this. Its concrete selected-I/current/close
    /// guard is mandatory at that call site; there is no Store-wide arbitrary-byte signer.
    pub(super) fn sign_reserved_append_bytes_data(
        &self,
        store: &ServerStore,
        original_plan: &catcoms_sync::owner_v2::V2AppendPlanObservation,
        prepared: &mut catcoms_sync::owner_v2::PreparedV2AppendData,
    ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
        match &self.core {
            OwnedCurrent::Warm(c) => c.sign_text_append_bytes(store, original_plan, prepared),
            OwnedCurrent::Cold(c) => c.sign_text_append_bytes(store, original_plan, prepared),
            OwnedCurrent::Repaired(c) => c.sign_text_append_bytes(store, original_plan, prepared),
        }
    }
    pub(super) fn materialize_signed_append_data(
        &self,
        store: &ServerStore,
        model: &catcoms_replication::chat::owner_state::transition::ReplayMath,
        prepared: &mut catcoms_sync::owner_v2::PreparedV2AppendData,
    ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
        match &self.core {
            OwnedCurrent::Warm(c) => c.materialize_text_append(store, model, prepared),
            OwnedCurrent::Cold(c) => c.materialize_text_append(store, model, prepared),
            OwnedCurrent::Repaired(c) => c.materialize_text_append(store, model, prepared),
        }
    }
    /// ORDINARY-member seams (never the founder Append/Header lane). Pre-admission: the
    /// SAME original node's full retained device key after its full source negatives.
    pub(super) fn ordinary_member_signer_data(
        &self,
        store: &ServerStore,
    ) -> Result<[u8; 32], catcoms_sync::owner_v2::V2MemberSignRefusal> {
        match &self.core {
            OwnedCurrent::Warm(c) => c.text_ordinary_member_signer(store),
            OwnedCurrent::Cold(c) => c.text_ordinary_member_signer(store),
            OwnedCurrent::Repaired(c) => c.text_ordinary_member_signer(store),
        }
    }
    /// Outside every writer: the request's one Clock/RNG draw through this node's seams.
    pub(super) fn draw_ordinary_draft_inputs(
        &mut self,
        store: &ServerStore,
        slot: &mut catcoms_sync::owner_v2::V2MemberDraftInputs,
    ) -> Result<(u64, [u8; 16]), catcoms_sync::owner_v2::V2MemberSignRefusal> {
        match &mut self.core {
            OwnedCurrent::Warm(c) => c.draw_text_ordinary_inputs(store, slot),
            OwnedCurrent::Cold(c) => c.draw_text_ordinary_inputs(store, slot),
            OwnedCurrent::Repaired(c) => c.draw_text_ordinary_inputs(store, slot),
        }
    }
    /// Only the member lane calls this, inside its Current/group guard with Core's original
    /// reader held by `guard`. The node caches raw64 in the Plan before returning.
    pub(super) fn sign_ordinary_admitted<'a>(
        &self,
        inputs: &catcoms_sync::owner_v2::V2MemberDraftInputs,
        guard: catcoms_replication::chat::V2OrdinarySigningGuard<'a>,
    ) -> Result<
        catcoms_replication::chat::V2OrdinarySigningGuard<'a>,
        catcoms_sync::owner_v2::V2MemberSignRefusal,
    > {
        match &self.core {
            OwnedCurrent::Warm(c) => c.sign_text_ordinary_admitted(inputs, guard),
            OwnedCurrent::Cold(c) => c.sign_text_ordinary_admitted(inputs, guard),
            OwnedCurrent::Repaired(c) => c.sign_text_ordinary_admitted(inputs, guard),
        }
    }
    pub(super) fn sign_ordinary_admitted_native<'a>(
        &self,
        inputs: &catcoms_sync::owner_v2::V2MemberDraftInputs,
        guard: catcoms_replication::chat::V2OrdinarySigningGuard<'a>,
        original: &catcoms_rt::NativeChatSourceObservation,
        cut: &mut catcoms_rt::NativeChatPreparedKeyCut,
        ends: &mut catcoms_rt::NativeChatKeyEnds,
    ) -> Result<
        catcoms_replication::chat::V2OrdinarySigningGuard<'a>,
        catcoms_sync::owner_v2::V2MemberSignRefusal,
    > {
        match &self.core {
            OwnedCurrent::Warm(c) => {
                c.sign_text_ordinary_admitted_native(inputs, guard, original, cut, ends)
            }
            OwnedCurrent::Cold(c) => {
                c.sign_text_ordinary_admitted_native(inputs, guard, original, cut, ends)
            }
            OwnedCurrent::Repaired(c) => {
                c.sign_text_ordinary_admitted_native(inputs, guard, original, cut, ends)
            }
        }
    }
    pub(super) fn borrow_unsigned_append<'a>(
        &'a self,
        store: &ServerStore,
        model: &'a catcoms_replication::chat::owner_state::transition::ReplayMath,
        prepared: &'a catcoms_sync::owner_v2::PreparedV2AppendData,
    ) -> Result<
        catcoms_sync::owner_v2::V2UnsignedAppendDataView<'a>,
        catcoms_sync::owner_v2::V2CreateRefusal,
    > {
        match &self.core {
            OwnedCurrent::Warm(c) => c.borrow_unsigned_text_append(store, model, prepared),
            OwnedCurrent::Cold(c) => c.borrow_unsigned_text_append(store, model, prepared),
            OwnedCurrent::Repaired(c) => c.borrow_unsigned_text_append(store, model, prepared),
        }
    }
}

/// Narrow DATA-only regression loans. These call the SAME actual family methods; they
/// expose neither a node/key nor raw signing admission and manufacture no saved intent.
#[cfg(all(test, windows))]
impl<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore> V2TextOwnedCurrent<T, R> {
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn prepare_create_data_for_test(
        &mut self,
        store: &ServerStore,
        input: catcoms_sync::owner_v2::V2CreateRequest<'_>,
        slot: &mut Option<catcoms_sync::owner_v2::PreparedV2CreateData>,
    ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
        self.prepare_create_data(store, input, slot)
    }
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn prepare_append_data_for_test(
        &self,
        store: &ServerStore,
        model: &catcoms_replication::chat::owner_state::transition::ReplayMath,
        action: &catcoms_replication::chat::SignedAction,
        slot: &mut Option<catcoms_sync::owner_v2::PreparedV2AppendData>,
    ) -> Result<(), catcoms_sync::owner_v2::V2CreateRefusal> {
        self.prepare_create_append_data(store, model, action, slot)
    }
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn borrow_unsigned_append_for_test<
        'a,
    >(
        &'a self,
        store: &ServerStore,
        model: &'a catcoms_replication::chat::owner_state::transition::ReplayMath,
        prepared: &'a catcoms_sync::owner_v2::PreparedV2AppendData,
    ) -> Result<
        catcoms_sync::owner_v2::V2UnsignedAppendDataView<'a>,
        catcoms_sync::owner_v2::V2CreateRefusal,
    > {
        self.borrow_unsigned_append(store, model, prepared)
    }
}

impl<'owner> V2TextOwnerRef<'owner> {
    /// Family-sealed cached association for Scope's actual existing row borrower. This
    /// remains a loan of the original owners, never an operational physical receipt.
    pub(in crate::store) fn original_association(&self) -> &GenesisCurrentOwnerRef<'owner> {
        &self.original
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'owner> {
        self.original.source()
    }
    pub(in crate::store) fn task(&self) -> &'owner PreparedV2GenesisGroupNamespace {
        self.original.task()
    }
    pub(in crate::store) fn warm_scope(&self) -> Option<&'owner V2CurrentPreparation> {
        self.original.warm_scope()
    }
    pub(in crate::store) fn cold_scope(&self) -> Option<&'owner ColdV2CurrentPreparation> {
        self.original.cold_scope()
    }
    pub(in crate::store) fn repaired_scope(&self) -> Option<&'owner ColdV2InitializingPreparation> {
        self.original.repaired_scope()
    }
    /// Clone only the original paid negative observation; no registration or sender.
    pub(in crate::store) fn observe_writer(&self) -> V2WriterObservation {
        self.original.observe_writer()
    }
    /// No admission/Root/pool/SDK/codec/FS operation, and no physical genesis validation.
    pub(in crate::store) fn require_original_negative(
        &self,
        store: &ServerStore,
        writer: &V2WriterObservation,
    ) -> Result<(), AppError> {
        self.original.require_original_negative(store, writer)
    }
    /// Historical DATA: the canonical `V2GroupPinId` of the EXACT authenticated original pin
    /// ciphertext retained by this owner's single real family (warm, cold or repaired). The
    /// existing typed capture labels the pin with this same identity. It stays computable for
    /// the whole owner life, after O advances beyond generation 1, so member-outbox receipts
    /// remain readable. Nothing is reopened, resealed or re-derived from the public Profile, and
    /// it grants no current OPEN, signing or save right.
    pub(in crate::store) fn original_pin_id_data(&self) -> Result<V2GroupPinId, AppError> {
        let pin = match (
            self.original.warm_head(),
            self.original.cold_head(),
            self.original.repaired_head(),
        ) {
            (Some(head), None, None) => head.text_pin_data()?.2,
            (None, Some(head), None) => head.text_pin_data()?.2,
            (None, None, Some(head)) => head.text_pin_data()?.2,
            _ => return Err(refused()),
        };
        Ok(V2GroupPinId::of(pin))
    }
    /// Recheck that exactly one real original family is present. Scope must not interpret
    /// optional family loans as a caller-selected role or fall through to another family.
    pub(in crate::store) fn require_closed_family(&self) -> Result<(), AppError> {
        match (self.warm_scope(), self.cold_scope(), self.repaired_scope()) {
            (Some(_), None, None) | (None, Some(_), None) | (None, None, Some(_)) => Ok(()),
            _ => Err(refused()),
        }
    }
}

impl<T, R> std::fmt::Debug for V2TextOwnedCurrent<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextOwnedCurrent").finish_non_exhaustive()
    }
}
impl std::fmt::Debug for V2TextOwnerRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextOwnerRef").finish_non_exhaustive()
    }
}
