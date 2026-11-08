//! Closed ownership adapters for the actual operational host.
//!
//! The original wrappers have meaningful Drop implementations. Keep them whole instead of
//! extracting a node, reconstructing an SDK, or cancelling an old owner during conversion.
//! This module performs cached association checks only. Operational physical selection is
//! proved separately under the new Scope/canonical/Root guards.

use crate::store::chat_admission::v2_current::{
    ColdV2CurrentPreparation, ColdV2InitializingPreparation, V2CurrentPreparation,
};
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

    /// New host metadata invoice includes this FULL closed layout, with no overlap credit
    /// from the independently retained old wrapper. This creates no allocation or payment.
    pub(super) const fn retained_layout_bytes() -> usize {
        std::mem::size_of::<Self>()
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
