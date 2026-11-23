//! Borrow only the warm Current owner's exact Remove primitives.
use super::*;
use catcoms_mls::persist::text_current_add::TextCurrentAddAdmission;
use catcoms_replication::chat::{
    owner_header::{OwnerBodyId, SignedOwnerHeader},
    V2GeneralCandidate, V2GeneralPrefixVerifier,
};
use catcoms_rt::{
    CryptoRngCore, MeshTransport, NativeChatKeyEnds, NativeChatPreparedKeyCut,
    NativeChatSourceObservation,
};
use catcoms_sync::owner_v2::{
    PreparedV2FounderRemoveInstall, RetiredV2FounderRemoveInstall, V2FounderRemoveOutcomeData,
    V2FounderRemoveSigning,
};

impl<T: MeshTransport, R: CryptoRngCore> V2TextOwnedCurrent<T, R> {
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current::operational) fn execute_governed_removal(
        &mut self,
        store: &ServerStore,
        verifier: &V2GeneralPrefixVerifier,
        target: OwnerBodyId,
        original: &NativeChatSourceObservation,
        payment: &mut Option<TextCurrentAddAdmission>,
        cut: &mut NativeChatPreparedKeyCut,
        ends: &mut NativeChatKeyEnds,
    ) -> Result<(), AppError> {
        self.warm_removal_mut()?
            .execute_governed_removal(store, verifier, target, original, payment, cut, ends)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current::operational) fn governed_removal_outcome(
        &self,
        store: &ServerStore,
    ) -> Result<V2FounderRemoveOutcomeData<'_>, AppError> {
        let OwnedCurrent::Warm(current) = &self.core else {
            return Err(refused());
        };
        current.governed_removal_outcome(store)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current::operational) fn borrow_removal_signing(
        &mut self,
        store: &ServerStore,
        verifier: &V2GeneralPrefixVerifier,
    ) -> Result<(V2TextOwnerRef<'_>, V2FounderRemoveSigning<'_>), AppError> {
        let current = self.warm_removal_mut()?;
        current
            .require_text_original_data(store)
            .map_err(|_| refused())?;
        let (original, primitive) =
            current.borrow_governed_parts(|node| node.removal_signing_parts_data(verifier))?;
        Ok((V2TextOwnerRef { original }, primitive))
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current::operational) fn materialize_governed_removal(
        &mut self,
        store: &ServerStore,
    ) -> Result<SignedOwnerHeader, AppError> {
        self.warm_removal_mut()?.materialize_governed_removal(store)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current::operational) fn derive_governed_removal_selection(
        &self,
        store: &ServerStore,
        verifier: &mut V2GeneralPrefixVerifier,
    ) -> Result<V2GeneralCandidate, AppError> {
        let OwnedCurrent::Warm(current) = &self.core else {
            return Err(refused());
        };
        current.derive_governed_removal_selection(store, verifier)
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current::operational) fn borrow_removal_install(
        &mut self,
        store: &ServerStore,
        verifier: &V2GeneralPrefixVerifier,
    ) -> Result<(V2TextOwnerRef<'_>, PreparedV2FounderRemoveInstall<'_>), AppError> {
        let current = self.warm_removal_mut()?;
        current
            .require_text_original_data(store)
            .map_err(|_| refused())?;
        let (original, primitive) =
            current.borrow_governed_parts(|node| node.prepare_removal_install_data(verifier))?;
        Ok((V2TextOwnerRef { original }, primitive))
    }

    pub(in crate::store::persistence::cold_source_read::genesis_seed::current::operational) fn take_retired_governed_removal(
        &mut self,
    ) -> Result<RetiredV2FounderRemoveInstall, AppError> {
        self.warm_removal_mut()?.take_retired_governed_removal()
    }

    fn warm_removal_mut(&mut self) -> Result<&mut UnexposedV2GenesisCurrent<T, R>, AppError> {
        match &mut self.core {
            OwnedCurrent::Warm(current) => Ok(current),
            OwnedCurrent::Cold(_) | OwnedCurrent::Repaired(_) => Err(refused()),
        }
    }
}
