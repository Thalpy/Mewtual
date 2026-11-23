//! Retain a cold founder's ordinary request and its native signing plan.
use super::*;
use crate::store::persistence::v2_member_outbox::MemberAppendAdmissionFacts;
use catcoms_replication::chat::ProjectionGroupView;
use catcoms_rt::{Clock, NativeChatKeyPurpose};
use catcoms_sync::owner_v2::{PreparedV2FounderRecoveredAdd, V2MemberPublicationObservation};

pub(in crate::store) struct RecoveredMemberLane {
    pub(in super::super) requests: MemberRequests,
    channels: Vec<MemberChannel>,
    save_source: V2MemberSaveSource,
    context: MemberOutboxContextId,
    original: NativeChatSourceObservation,
    fee: CatalogReadReservation,
}

pub(in crate::store) struct RecoveredMemberSource<'a, R> {
    pub(in crate::store) sdk: &'a PreparedV2FounderRecoveredAdd,
    pub(in crate::store) verifier: &'a V2GeneralPrefixVerifier,
    pub(in crate::store) view: &'a ProjectionGroupView,
    pub(in crate::store) cut: &'a mut NativeChatPreparedKeyCut,
    pub(in crate::store) ends: &'a mut NativeChatKeyEnds,
    pub(in crate::store) clock: &'a dyn Clock,
    pub(in crate::store) rng: &'a mut R,
    pub(in crate::store) require_files: &'a mut dyn FnMut() -> Result<(), AppError>,
}

impl RecoveredMemberLane {
    pub(in crate::store) fn new(
        store: &ServerStore,
        context: MemberOutboxContextId,
        original: NativeChatSourceObservation,
        selected: V2MemberPublicationObservation,
    ) -> Result<Self, AppError> {
        if !selected.is_current_cached() {
            return Err(refused());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(V2_MEMBER_LANE_BYTES)?;
        let save_source = V2MemberSaveSource::prepare_recovered_founder(pool, selected)?;
        Ok(Self {
            requests: MemberRequests::new(),
            channels: Vec::with_capacity(MAX_MEMBER_CHANNELS),
            save_source,
            context,
            original,
            fee,
        })
    }

    pub(in crate::store) fn context(&self) -> MemberOutboxContextId {
        self.context
    }

    pub(in crate::store) fn require_cut(
        &self,
        cut: &NativeChatPreparedKeyCut,
    ) -> Result<(), AppError> {
        cut.preflight(&self.original, NativeChatKeyPurpose::MemberAction)
            .map_err(|_| refused())
    }

    pub(in super::super) fn admit_save_source(
        &self,
        facts: MemberAppendAdmissionFacts,
        sdk: &PreparedV2FounderRecoveredAdd,
        cut: &NativeChatPreparedKeyCut,
    ) -> Result<V2MemberSaveSourceEnd, AppError> {
        self.require_cut(cut)?;
        if facts.context != self.context {
            return Err(refused());
        }
        let pending =
            self.requests
                .require_lent(*facts.token.as_bytes(), facts.action, facts.admission)?;
        let plan = pending.plan.as_ref().ok_or_else(refused)?;
        sdk.require_member_save_plan_cached(plan)
            .map_err(|_| refused())?;
        self.save_source.issue()
    }

    pub(in crate::store) fn require_retained(
        &self,
        token: [u8; 32],
        original: &MemberSignedOriginal,
    ) -> Result<(), AppError> {
        self.requests
            .require_lent(token, original.action().id(), original.admission())?;
        Ok(())
    }

    pub(in crate::store) fn signer<R>(
        &self,
        source: &RecoveredMemberSource<'_, R>,
    ) -> Result<[u8; 32], AppError> {
        source
            .sdk
            .member_signer_data(source.verifier, &self.original, source.cut)
            .map_err(|_| refused())
    }

    pub(in crate::store) fn retry_request<R: CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        request: &V2MemberSignRequest<'_>,
        source: &mut RecoveredMemberSource<'_, R>,
    ) -> Result<(), AppError> {
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        self.fee.validate(pool, V2_MEMBER_LANE_BYTES)?;
        self.require_cut(source.cut)?;
        (source.require_files)()?;
        if self.requests.retry_completed(request)? {
            return Ok(());
        }
        let signer = self.signer(source)?;
        self.requests.ensure_pending(store, request)?;
        let pending = self.requests.request.as_deref_mut().ok_or_else(refused)?;
        pending.validate_accounts(store)?;
        let own = source
            .sdk
            .selected_member_admission(source.verifier)
            .map_err(|_| refused())?;
        let group = &source.verifier.replay_data().profile().spec().group;
        if pending.draft.is_none() && pending.inputs.drawn_data().is_none() {
            source
                .sdk
                .draw_member_draft_inputs(
                    source.verifier,
                    &self.original,
                    source.cut,
                    &mut pending.inputs,
                    source.clock,
                    source.rng,
                )
                .map_err(|_| refused())?;
        }
        if pending.draft.is_none() {
            pending.prepare_draft(group, signer, own)?;
        }
        if pending.plan.is_none() {
            let view =
                member_channel_view(&mut self.channels, group, pending.channel, source.view)?;
            pending.prepare_plan(source.verifier, own, request.dependencies, view)?;
        }
        pending.sign_and_complete(|inputs, draft, plan| {
            (source.require_files)()?;
            source
                .sdk
                .sign_member_action_native(
                    source.verifier,
                    inputs,
                    draft,
                    plan,
                    &self.original,
                    source.cut,
                    source.ends,
                )
                .map(drop)
                .map_err(|_| refused())
        })
    }
}

impl Drop for RecoveredMemberLane {
    fn drop(&mut self) {
        self.save_source.close();
    }
}
