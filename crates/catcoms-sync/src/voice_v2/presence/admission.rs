//! Reserve only against this actual LocalJoin. Public labels are not pool identity.
use super::*;
use crate::voice_v2::admission::{
    AdmissionSessionObserver, Preparation, RootAdmissionSession, RootHighPreclaim,
};
use crate::voice_v2::runtime::ActorOwner;

impl LocalJoin {
    pub(crate) fn prepare_admission_session(
        &self,
        actor: &ActorOwner,
        context: &Context,
        watch: &WatchLease,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        before_final: impl FnOnce(),
    ) -> Result<RootAdmissionSession, VoiceError> {
        if self.scope != context.scope || self.local != context.local {
            return Err(VoiceError::Unauthorized);
        }
        let slots = self.ledger.action_slots();
        RootAdmissionSession::prepare(
            Preparation {
                actor,
                context,
                resources: &self.resources,
                slots: &slots,
                call: &self.lease,
                watch,
                key: &self.key,
            },
            rng,
            before_final,
        )
    }

    pub(crate) fn reserve_high_admission_preclaim(
        &self,
        actor: &ActorOwner,
        context: &Context,
        watch: &WatchLease,
        original: &AdmissionSessionObserver,
        generation: u64,
    ) -> Result<RootHighPreclaim, VoiceError> {
        if self.scope != context.scope || self.local != context.local {
            return Err(VoiceError::Unauthorized);
        }
        let slots = self.ledger.action_slots();
        original.reserve_high(
            Preparation {
                actor,
                context,
                resources: &self.resources,
                slots: &slots,
                call: &self.lease,
                watch,
                key: &self.key,
            },
            self.ledger.packet_slots(),
            generation,
        )
    }
}
