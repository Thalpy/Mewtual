//! Use the recovered founder in the same two-phase ordinary outbox.
use super::*;
use crate::store::persistence::{RecoveredMemberLane, RecoveredMemberSource};
use crate::store::{
    MemberSaveJob, MemberSaveNativeAdmission, V2MemberSaveOutcome, V2MemberSaveRefusal as Refusal,
    V2MemberSignRequest,
};
use catcoms_rt::{
    Clock, NativeChatKeyEnds, NativeChatKeyPurpose, NativeChatPreparedKeyCut,
    NativeChatSourceObservation,
};

#[cfg(test)]
use catcoms_rt::{NativeChatSourceOwner, NativeChatUiCommand};

#[cfg(test)]
pub(in crate::store::critical_namespaces::native_registry) struct FounderSavePreparation {
    original: Arc<crate::store::persistence::native_registry::network::NetworkDependency>,
    token: [u8; 32],
    cut: NativeChatPreparedKeyCut,
    signal: RequestCancellation,
}

impl NativeTextColdFounderSource {
    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn prepare_member_save<
        R: CryptoRngCore,
    >(
        &mut self,
        request: V2MemberSignRequest<'_>,
        owner: &NativeChatSourceOwner,
        command: &NativeChatUiCommand,
        caller: &RequestCancellation,
        runtime: (&dyn Clock, &mut R),
        outbox_rng: &mut impl CryptoRngCore,
    ) -> Result<V2MemberSaveOutcome, Refusal> {
        let mut cut = owner
            .prepare_cut(command, NativeChatKeyPurpose::MemberAction)
            .map_err(|_| Refusal::Unavailable)?;
        let mut ends = NativeChatKeyEnds::new();
        self.prepare_member_save_with_keys(
            request,
            (&owner.observe(), &mut cut, &mut ends),
            caller,
            runtime,
            outbox_rng,
        )
    }

    pub(in crate::store::critical_namespaces::native_registry) fn prepare_member_save_with_keys<
        R: CryptoRngCore,
    >(
        &mut self,
        request: V2MemberSignRequest<'_>,
        keys: (
            &NativeChatSourceObservation,
            &mut NativeChatPreparedKeyCut,
            &mut NativeChatKeyEnds,
        ),
        caller: &RequestCancellation,
        runtime: (&dyn Clock, &mut R),
        outbox_rng: &mut impl CryptoRngCore,
    ) -> Result<V2MemberSaveOutcome, Refusal> {
        if self.index.is_some() {
            return Err(Refusal::Unavailable);
        }
        if caller.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        let (native, cut, ends) = keys;
        cut.preflight(native, NativeChatKeyPurpose::MemberAction)
            .map_err(|_| Refusal::Unavailable)?;
        if self.member.is_none() {
            if let Some((action, ordinal)) =
                self.lookup_saved_member(&request, caller, outbox_rng)?
            {
                return Ok(V2MemberSaveOutcome::Saved { action, ordinal });
            }
        }
        self.restore_first_add(caller).map_err(|_| Refusal::Stale)?;
        let context = self.outbox_context().map_err(|_| Refusal::Stale)?;
        let governed = self.governed.as_deref_mut().ok_or(Refusal::Unavailable)?;
        let sdk = self.sdk.as_mut().ok_or(Refusal::Unavailable)?;
        let member = &mut self.member;
        let guarded = self
            .work
            .with_recovered_founder_layout(&self.selection, &self.source, |layout, store| {
                governed.with_checked_member_parts(
                    store,
                    &self.source,
                    |verifier, view, require_files| {
                        sdk.bind_native_member_source(verifier, native, cut)
                            .map_err(|_| refused())?;
                        if member.is_none() {
                            let (_, selected) = sdk.prepare_member_topic(
                                verifier,
                                request.channel,
                                store.chat_mount.observer(),
                            )?;
                            *member = Some(Box::new(RecoveredMemberLane::new(
                                store,
                                context.id(),
                                native.clone(),
                                selected,
                            )?));
                        }
                        let lane = member.as_deref_mut().ok_or_else(refused)?;
                        let mut require_current = || {
                            require_files()?;
                            if caller.is_cancelled() {
                                return Err(refused());
                            }
                            Ok(())
                        };
                        let source = RecoveredMemberSource {
                            sdk,
                            verifier,
                            view,
                            cut: &mut *cut,
                            ends: &mut *ends,
                            clock: runtime.0,
                            rng: runtime.1,
                            require_files: &mut require_current,
                        };
                        Ok(lane.prepare_member_save(
                            layout, request, source, context, caller, outbox_rng,
                        ))
                    },
                )
            })
            .map_err(|_| Refusal::Unavailable)?;
        if caller.is_cancelled() || cut.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        self.restore_first_add(caller).map_err(|_| Refusal::Stale)?;
        guarded
    }

    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn finish_member_save(
        &mut self,
        token: [u8; 32],
        owner: &NativeChatSourceOwner,
        command: &NativeChatUiCommand,
        caller: RequestCancellation,
    ) -> Result<FounderSavePreparation, Refusal> {
        self.restore_first_add(&caller)
            .map_err(|_| Refusal::Stale)?;
        let lane = self.member.as_ref().ok_or(Refusal::Stale)?;
        let cut = owner
            .prepare_cut(command, NativeChatKeyPurpose::MemberAction)
            .map_err(|_| Refusal::Unavailable)?;
        lane.require_cut(&cut).map_err(|_| Refusal::Unavailable)?;
        Ok(FounderSavePreparation {
            original: self.selection.network.clone(),
            token,
            cut,
            signal: caller,
        })
    }

    pub(in crate::store::critical_namespaces::native_registry) fn require_member_save_admission(
        &mut self,
        cut: &NativeChatPreparedKeyCut,
        caller: &RequestCancellation,
    ) -> Result<(), Refusal> {
        self.restore_first_add(caller).map_err(|_| Refusal::Stale)?;
        self.member
            .as_ref()
            .ok_or(Refusal::Stale)?
            .require_cut(cut)
            .map_err(|_| Refusal::Unavailable)
    }

    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn admit_member_save<
        R: CryptoRngCore,
    >(
        &self,
        preparation: &FounderSavePreparation,
        native: MemberSaveNativeAdmission,
    ) -> Result<MemberSaveJob, (MemberSaveNativeAdmission, Refusal)> {
        if !Arc::ptr_eq(&preparation.original, &self.selection.network) {
            return Err((native, Refusal::Stale));
        }
        if preparation.signal.is_cancelled() {
            return Err((native, Refusal::Cancelled));
        }
        self.admit_restored_member_save::<R>(preparation.token, &preparation.cut, native)
    }

    pub(in crate::store::critical_namespaces::native_registry) fn admit_restored_member_save<
        R: CryptoRngCore,
    >(
        &self,
        token: [u8; 32],
        cut: &NativeChatPreparedKeyCut,
        native: MemberSaveNativeAdmission,
    ) -> Result<MemberSaveJob, (MemberSaveNativeAdmission, Refusal)> {
        let (Some(lane), Some(sdk), Ok(layout)) = (
            self.member.as_ref(),
            self.sdk.as_ref(),
            self.work.cached_founder_save_layout(),
        ) else {
            return Err((native, Refusal::Unavailable));
        };
        lane.admit_member_save::<R>(layout, token, sdk, cut, native)
    }
}
