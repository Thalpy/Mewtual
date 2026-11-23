//! Ordinary action key admission on the exact recovered founder graph.
use super::*;
use crate::owner_v2::member::{sign_admitted_with_device_native, NativeMemberSigning};
use crate::owner_v2::{V2MemberDraftInputs, V2MemberSignRefusal};
use catcoms_replication::chat::{
    ActionDraft, V2GeneralPrefixVerifier, V2OrdinarySigningGuard, V2OrdinarySigningPlan,
};
use catcoms_rt::{
    Clock, NativeChatKeyEnds, NativeChatKeyPurpose, NativeChatPreparedKeyCut,
    NativeChatSourceObservation,
};

impl PreparedV2FounderRecoveredAdd {
    /// Bind the first genuine native MemberAction cut after the host proves
    /// its physical selection and installs a fresh authenticated Source.
    pub fn bind_native_member_source(
        &mut self,
        verifier: &V2GeneralPrefixVerifier,
        original: &NativeChatSourceObservation,
        cut: &NativeChatPreparedKeyCut,
    ) -> Result<(), V2MemberSignRefusal> {
        self.bound_member(verifier)
            .map_err(|_| V2MemberSignRefusal::Source)?;
        cut.preflight(original, NativeChatKeyPurpose::MemberAction)
            .map_err(V2MemberSignRefusal::Native)?;
        match self.native_origin.as_ref() {
            Some(bound) if bound.same_original(original) => Ok(()),
            Some(_) => Err(V2MemberSignRefusal::Source),
            None => {
                self.native_origin = Some(original.clone());
                Ok(())
            }
        }
    }

    pub fn member_signer_data(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        original: &NativeChatSourceObservation,
        cut: &NativeChatPreparedKeyCut,
    ) -> Result<[u8; 32], V2MemberSignRefusal> {
        self.bound_member(verifier)
            .map_err(|_| V2MemberSignRefusal::Source)?;
        if !self
            .native_origin
            .as_ref()
            .is_some_and(|bound| bound.same_original(original))
        {
            return Err(V2MemberSignRefusal::Source);
        }
        cut.preflight(original, NativeChatKeyPurpose::MemberAction)
            .map_err(V2MemberSignRefusal::Native)?;
        Ok(self.signer)
    }

    /// The native owner supplies its clock and RNG. Each result belongs to
    /// the saved request's once-only draw slot on this original restored SDK.
    pub fn draw_member_draft_inputs(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        original: &NativeChatSourceObservation,
        cut: &NativeChatPreparedKeyCut,
        inputs: &mut V2MemberDraftInputs,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(u64, [u8; 16]), V2MemberSignRefusal> {
        self.member_signer_data(verifier, original, cut)?;
        inputs.bind_recovered_founder_origin(&self.lifetime)?;
        if inputs.enter_clock()? {
            inputs.cache_time(clock.now_ms());
        }
        self.member_signer_data(verifier, original, cut)?;
        if inputs.enter_nonce()? {
            let mut nonce = [0; 16];
            rng.fill_bytes(&mut nonce);
            inputs.cache_nonce(nonce);
        }
        self.member_signer_data(verifier, original, cut)?;
        inputs.drawn()
    }

    /// Native-only use of the original retained signing key. The Core
    /// Admission must match this verifier's exact OPEN lifetime.
    pub fn sign_member_action_native<'a>(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        inputs: &V2MemberDraftInputs,
        draft: &'a ActionDraft,
        plan: &'a mut V2OrdinarySigningPlan,
        original: &NativeChatSourceObservation,
        cut: &mut NativeChatPreparedKeyCut,
        ends: &mut NativeChatKeyEnds,
    ) -> Result<V2OrdinarySigningGuard<'a>, V2MemberSignRefusal> {
        let signer = self.member_signer_data(verifier, original, cut)?;
        inputs.require_recovered_founder_origin(&self.lifetime)?;
        let selected = self
            .bound_member(verifier)
            .map_err(|_| V2MemberSignRefusal::Source)?;
        plan.require_original_selected_admission(selected)
            .map_err(|_| V2MemberSignRefusal::Source)?;
        let guard = plan
            .admit_signing(draft, &signer)
            .map_err(|_| V2MemberSignRefusal::Source)?;
        sign_admitted_with_device_native(
            &self.device,
            inputs,
            guard,
            NativeMemberSigning {
                original,
                cut,
                ends,
            },
        )
    }
}
