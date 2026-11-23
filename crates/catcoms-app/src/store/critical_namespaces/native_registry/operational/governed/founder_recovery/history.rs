//! Rebuild one archived projection from the cold founder's selected history.
use super::*;
use crate::store::persistence::v2_member_outbox::{
    member_outbox_key, MemberHistoryPage, MemberHistoryRead, MemberOutboxContextData,
    MemberOutboxContextSpec, MemberRequestData, MemberRequestToken, MemberSavedOriginal,
};
use crate::store::persistence::v2_member_transport::{
    FOUNDER_RECEIVE_ACTIONS as ACTION_LIMIT, FOUNDER_RECEIVE_BYTES as ACTION_BYTES,
    FOUNDER_RECEIVE_LEDGER_FEE as LEDGER_FEE, FOUNDER_RECEIVE_WORK as WORK_LIMIT,
};
use crate::store::persistence::v2_shared_history::{
    SharedHistoryActionQuery, SharedHistoryProjectionLease, SharedHistoryReplay, SharedHistoryScope,
};
use crate::store::{
    NativeTextJoinedArchivedActions, NativeTextJoinedMemberProjection, ServerStore,
    V2MemberSaveRefusal, V2MemberSignRequest,
};
use catcoms_replication::chat::{ActionId, ChatLedger, LedgerLimits, Realm, MAX_ACTION_BYTES};

impl NativeTextColdFounderSource {
    pub(in crate::store::critical_namespaces::native_registry) fn read_own_history(
        &mut self,
        request: MemberHistoryRead,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<MemberHistoryPage, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        self.restore_first_add(caller).map_err(|_| Refusal::Stale)?;
        let result = self
            .work
            .with_recovered_founder_layout(&self.selection, &self.source, |layout, _| {
                let context = outbox_context_spec(&self.selection, &self.source);
                Ok(
                    crate::store::persistence::prepare_recovered_founder_history(
                        layout, request, caller, rng, context,
                    ),
                )
            })
            .map_err(|_| Refusal::Unavailable)?;
        self.restore_first_add(caller).map_err(|_| Refusal::Stale)?;
        result
    }

    /// Resolve an exact retry against the existing authenticated founder outbox
    /// before preparing a signed action. This is independent of TCP recovery.
    pub fn lookup_saved_member(
        &mut self,
        request: &V2MemberSignRequest<'_>,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<(ActionId, u64)>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let require_caller = || {
            if caller.is_cancelled() {
                return Err(Refusal::Cancelled);
            }
            Ok(())
        };
        require_caller()?;
        self.restore_first_add(caller).map_err(|_| Refusal::Stale)?;
        let context = self.outbox_context().map_err(|_| Refusal::Stale)?;
        let original = &self.source;
        let founder = original
            .data()
            .genesis_public_data()
            .admission_data()
            .admission_id();
        let receipt = self
            .work
            .with_recovered_founder_store(&self.selection, original, |store| {
                let mount = store.member_outbox_mount()?;
                let key = member_outbox_key(store)?;
                Ok(mount.lookup_existing(
                    store,
                    context,
                    &key,
                    &MemberRequestData {
                        token: MemberRequestToken::from_bytes(request.token),
                        channel: request.channel,
                        body: request.body,
                        parents: request.parents,
                    },
                    rng,
                ))
            })
            .map_err(|_| Refusal::Failed)?;
        require_caller()?;
        self.restore_first_add(caller).map_err(|_| Refusal::Stale)?;
        let receipt = receipt.map_err(|error| Refusal::from(&error))?;
        if receipt
            .as_ref()
            .is_some_and(|value| value.admission != founder)
        {
            return Err(Refusal::Stale);
        }
        Ok(receipt.map(|value| (value.action, value.ordinal)))
    }

    pub fn archived_action_page(
        &mut self,
        channel: u128,
        after: Option<ActionId>,
        limit: usize,
        caller: &RequestCancellation,
    ) -> Result<Option<NativeTextJoinedArchivedActions>, AppError> {
        self.restore_first_add(caller)?;
        let scope = self.history_scope(channel)?;
        let result =
            self.work
                .with_recovered_founder_store(&self.selection, &self.source, |store| {
                    if !store.has_mounted_shared_history()? {
                        return Ok(None);
                    }
                    store
                        .shared_history_action_page(
                            scope,
                            SharedHistoryActionQuery {
                                target: None,
                                after,
                                limit,
                            },
                        )
                        .map(NativeTextJoinedArchivedActions)
                        .map(Some)
                })?;
        self.restore_first_add(caller)?;
        Ok(result)
    }

    pub fn archived_message_projection(
        &mut self,
        channel: u128,
        create: ActionId,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<NativeTextJoinedMemberProjection>, AppError> {
        self.restore_first_add(caller)?;
        let scope = self.history_scope(channel)?;
        let (verifier, group_view) = self.governed.as_ref().ok_or_else(refused)?.member_parts()?;
        let sdk = self.sdk.as_ref().ok_or_else(refused)?;
        let original = &self.source;
        let result =
            self.work
                .with_recovered_founder_store(&self.selection, &self.source, |store| {
                    if !store.has_mounted_shared_history()? {
                        return Ok(None);
                    }
                    let pool = &store
                        .archive_bootstrap_policy
                        .as_ref()
                        .ok_or_else(refused)?
                        .pending
                        .pool;
                    let fee = pool.try_reserve(LEDGER_FEE)?;
                    let (_, observation) =
                        sdk.prepare_member_topic(verifier, channel, store.chat_mount.observer())?;
                    let signal = observation.constrain(caller.clone());
                    let realm = Realm::new(&scope.group, channel).map_err(|_| refused())?;
                    let mut ledger = ChatLedger::new(
                        realm,
                        LedgerLimits {
                            max_actions: ACTION_LIMIT,
                            max_action_bytes: ACTION_BYTES,
                        },
                    )
                    .map_err(|_| refused())?;
                    let mut view = ledger
                        .begin_projection_view_in(group_view)
                        .map_err(|_| refused())?;
                    if !view.try_ready() {
                        return Err(refused());
                    }
                    let original = SharedHistoryReplay {
                        store,
                        scope,
                        verifier,
                        caller: &signal,
                    }
                    .replay_message(&mut ledger, &view, create, |id| {
                        saved_dependency(store, self.outbox_context(), original, scope, id, rng)
                    })
                    .map_err(|_| refused())?;
                    let Some(observation) = original else {
                        return Ok(None);
                    };
                    let projection_fee = pool.try_reserve(2 * MAX_ACTION_BYTES + 32 * 1024)?;
                    let projection = ledger
                        .guarded_projection(create, WORK_LIMIT)
                        .map_err(|_| refused())?;
                    Ok(Some(NativeTextJoinedMemberProjection::from_archive(
                        projection,
                        signal,
                        projection_fee,
                        Box::new(SharedHistoryProjectionLease {
                            observation,
                            _view: view,
                            _ledger: ledger,
                            _invocation: None,
                            _fee: fee,
                        }),
                    )))
                })?;
        self.restore_first_add(caller)?;
        Ok(result)
    }

    pub(super) fn history_scope(&self, channel: u128) -> Result<SharedHistoryScope, AppError> {
        let original = self.source.data().genesis_public_data();
        let profile = original.profile_data();
        Ok(SharedHistoryScope {
            vault: self.selection.binding.vault,
            group: self
                .selection
                .binding
                .group
                .as_slice()
                .try_into()
                .map_err(|_| refused())?,
            profile: profile.id(),
            tenure: profile.tenure(),
            creation: profile.spec().creation,
            channel,
        })
    }

    pub(super) fn outbox_context(&self) -> Result<MemberOutboxContextData, AppError> {
        outbox_context(&self.selection, &self.source)
    }
}

pub(super) fn outbox_context(
    selection: &ColdFounderSelection,
    source: &RetainedBootstrapSourceData,
) -> Result<MemberOutboxContextData, AppError> {
    MemberOutboxContextData::from_spec(outbox_context_spec(selection, source))
}

fn outbox_context_spec<'a>(
    selection: &'a ColdFounderSelection,
    source: &RetainedBootstrapSourceData,
) -> MemberOutboxContextSpec<'a> {
    let genesis = source.data().genesis_public_data();
    let profile = genesis.profile_data();
    let original = &selection.binding;
    MemberOutboxContextSpec {
        vault: original.vault,
        layout: original.layout,
        pin: original.pin,
        profile: profile.id(),
        tenure: profile.tenure(),
        group_nonce: profile.spec().creation,
        device: original.signer,
        group: &original.group,
    }
}

pub(super) fn saved_dependency(
    store: &ServerStore,
    context: Result<MemberOutboxContextData, AppError>,
    original: &RetainedBootstrapSourceData,
    scope: SharedHistoryScope,
    id: ActionId,
    rng: &mut impl CryptoRngCore,
) -> Result<Option<MemberSavedOriginal>, V2MemberSaveRefusal> {
    use V2MemberSaveRefusal as Refusal;
    let founder = original
        .data()
        .genesis_public_data()
        .admission_data()
        .admission_id();
    let mount = store
        .member_outbox_mount()
        .map_err(|_| Refusal::Unavailable)?;
    let key = member_outbox_key(store).map_err(|_| Refusal::Unavailable)?;
    let saved = mount
        .lookup_existing_action(store, context.map_err(|_| Refusal::Stale)?, &key, id, rng)
        .map_err(|_| Refusal::Failed)?;
    if saved.as_ref().is_some_and(|saved| {
        let realm = &saved.action().spec().realm;
        saved.receipt().admission != founder
            || realm.channel() != scope.channel
            || realm.group() != scope.group.as_slice()
    }) {
        return Err(Refusal::Stale);
    }
    Ok(saved)
}
