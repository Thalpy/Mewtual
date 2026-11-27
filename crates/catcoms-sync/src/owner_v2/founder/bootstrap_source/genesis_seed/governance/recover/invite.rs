//! Invitation DATA from the actual recovered selected SDK, for Root's
//! ordinary per-parent intent and signing journal.
use super::super::super::invite::{
    InviteOrigin, PreparedV2FounderInviteData, V2FounderInviteRequestData,
};
use super::*;
use catcoms_replication::chat::V2GeneralPrefixVerifier;
use catcoms_rt::{
    NativeChatKeyEnds, NativeChatKeyPurpose, NativeChatPreparedKeyCut, NativeChatSourceObservation,
};

impl PreparedV2FounderRecoveredAdd {
    /// The original policy available to reconstruct an already covered invite.
    /// Root authenticates that invite against its held intent and raw result.
    pub fn read_recovered_invite_policy(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        original: &OpenedBootstrapSourceData,
        native: &NativeChatSourceObservation,
        cut: &NativeChatPreparedKeyCut,
    ) -> Result<Vec<u8>, SyncError> {
        self.require_invite_source(verifier, original, native, cut)?;
        original.recovered_invite_policy_data()
    }

    fn require_invite_source(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        original: &OpenedBootstrapSourceData,
        native: &NativeChatSourceObservation,
        cut: &NativeChatPreparedKeyCut,
    ) -> Result<(), SyncError> {
        if self.source != original.source_id_data()
            || !self.group.is_designated_committer(&self.device)
            || self
                .native_origin
                .as_ref()
                .is_some_and(|bound| !bound.same_original(native))
        {
            return Err(unavailable());
        }
        self.bound_member(verifier)?;
        cut.preflight(native, NativeChatKeyPurpose::JoinInvite)
            .map_err(|_| unavailable())
    }

    /// Prepare the same signed-invite format as the warm founder using this
    /// independently restored key, selected parent and original S0 policy.
    /// Root must check the per-parent journal before permitting new draws.
    pub fn prepare_recovered_invite_data(
        &mut self,
        verifier: &V2GeneralPrefixVerifier,
        original: &OpenedBootstrapSourceData,
        request: V2FounderInviteRequestData,
        keys: (&NativeChatSourceObservation, &NativeChatPreparedKeyCut),
        runtime: (&dyn Clock, &mut impl CryptoRngCore),
        slot: &mut Option<PreparedV2FounderInviteData>,
    ) -> Result<(), SyncError> {
        request.validate()?;
        let (native, cut) = keys;
        self.require_invite_source(verifier, original, native, cut)?;
        let parent = (self.expected.epoch, self.expected.header);
        if let Some(prepared) = slot.as_ref() {
            prepared.require_recovered(parent, &self.lifetime, native)?;
            if prepared.request_data() != request {
                return Err(unavailable());
            }
        }
        if self.native_origin.is_none() {
            self.native_origin = Some(native.clone());
        }
        let prepared = slot.get_or_insert_with(|| {
            PreparedV2FounderInviteData::captured(
                request,
                Some(parent),
                InviteOrigin::Recovered(Arc::downgrade(&self.lifetime)),
                native,
            )
        });
        prepared.draw_and_encode(runtime.0, runtime.1, || {
            let genesis = original.genesis_public_data();
            let profile = genesis.profile_data();
            let header = genesis.header_data();
            Ok((
                GenesisAnchorBindingData {
                    group: profile.spec().group.clone(),
                    founder: profile.spec().founder,
                    profile: *profile.id().as_bytes(),
                    admission: *genesis.admission_data().admission_id().as_bytes(),
                    tenure: *profile.tenure().as_bytes(),
                    header: *header.id().as_bytes(),
                    state: *header.spec().state.as_bytes(),
                },
                original.recovered_invite_policy_data()?,
            ))
        })?;
        self.require_invite_source(verifier, original, native, cut)
    }

    /// Enter only the original prepared invitation and actual JoinInvite
    /// cut. Root covers the intent and key attempt before calling this edge.
    pub fn sign_recovered_invite_native(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        original: &OpenedBootstrapSourceData,
        prepared: &mut PreparedV2FounderInviteData,
        (native, cut, ends): (
            &NativeChatSourceObservation,
            &mut NativeChatPreparedKeyCut,
            &mut NativeChatKeyEnds,
        ),
    ) -> Result<(), SyncError> {
        self.require_invite_source(verifier, original, native, cut)?;
        prepared.require_recovered(
            (self.expected.epoch, self.expected.header),
            &self.lifetime,
            native,
        )?;
        prepared.sign_retained_device(&self.device, self.signer, cut, ends)
    }
}
