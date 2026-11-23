//! Invitation signing on the original founder after a selected Add.
use super::super::invite::{PreparedV2FounderInviteData, V2FounderInviteRequestData};
use super::*;
use catcoms_replication::chat::V2GeneralPrefixVerifier;

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2FounderGovernanceNode<T, R> {
    /// Freeze a fresh request against this actual selected Core and SDK parent.
    /// The host still owns the original native source and its durable intent.
    pub fn prepare_selected_invite_data(
        &mut self,
        verifier: &V2GeneralPrefixVerifier,
        request: V2FounderInviteRequestData,
        native: &NativeChatSourceObservation,
        slot: &mut Option<PreparedV2FounderInviteData>,
    ) -> Result<(), SyncError> {
        self.require_sdk_current_data()?;
        let parent = self.invite_parent(verifier)?;
        self.node
            .prepare_invite_for_parent(request, native, slot, Some(parent))?;
        let (spec, _) = slot.as_ref().ok_or_else(unavailable)?.unsigned_data()?;
        self.available_invite_slot(spec.nonce)
            .ok_or_else(unavailable)?;
        Ok(())
    }

    /// Full SDK validation before the host enters its Current writer.
    pub fn preflight_selected_invite_signing(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        prepared: &PreparedV2FounderInviteData,
    ) -> Result<(), SyncError> {
        self.require_sdk_current_data()?;
        let parent = self.invite_parent(verifier)?;
        self.node
            .require_invite_intent_parent(prepared, Some(parent))?;
        let (spec, _) = prepared.unsigned_data()?;
        self.available_invite_slot(spec.nonce)
            .ok_or_else(unavailable)?;
        Ok(())
    }

    /// Enter only for the original prepared intent and the still-selected
    /// parent. The host holds its matching original Current and native cut.
    pub fn sign_selected_invite_native(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        prepared: &mut PreparedV2FounderInviteData,
        cut: &mut NativeChatPreparedKeyCut,
        ends: &mut NativeChatKeyEnds,
    ) -> Result<(), SyncError> {
        let parent = self.invite_parent(verifier)?;
        let (spec, _) = prepared.unsigned_data()?;
        self.available_invite_slot(spec.nonce)
            .ok_or_else(unavailable)?;
        self.node
            .sign_invite_for_parent(prepared, Some(parent), cut, ends)
    }

    fn invite_parent(
        &self,
        verifier: &V2GeneralPrefixVerifier,
    ) -> Result<(u64, OwnerHeaderId), SyncError> {
        let installed = self.installed.ok_or_else(unavailable)?;
        let member = self.member.as_ref().ok_or_else(unavailable)?;
        let replay = verifier.replay_data();
        if self.pending.is_some()
            || replay.header().id() != installed.header
            || replay.header().spec().state != installed.state
            || replay.mls_epoch_data() != installed.epoch
            || !member.is_open_data()
            || !member.same_original(
                &verifier
                    .admission(member.admission_data().admission_id())
                    .map_err(|_| unavailable())?,
            )
        {
            return Err(unavailable());
        }
        Ok((installed.epoch, installed.header))
    }
}
