//! The founder uses the original selected Profile and vault for shared history.
use super::*;
use crate::store::critical_namespaces::read_pool::CriticalCatalogReadPool;
use crate::store::persistence::v2_member_outbox::{MemberOutboxMount, MemberSavedOriginal};
use crate::store::persistence::v2_member_transport::{
    archive_founder_member, FounderArchivedMessage,
};
use crate::store::persistence::v2_shared_history::{
    SharedHistoryActionPage, SharedHistoryProjectionLease, SharedHistoryReplay,
    SharedHistoryReplayCredit, SharedHistoryScope, ARCHIVED_LEDGER_LIMITS,
    ARCHIVED_PROJECTION_WORK,
};
use catcoms_sync::owner_v2::SelectedV2MemberEnvelope;

pub(super) struct FounderHistory<'a> {
    store: &'a ServerStore,
    verifier: &'a V2GeneralPrefixVerifier,
    owner: V2TextOwnerRef<'a>,
    pub(super) scope: SharedHistoryScope,
    admission: OwnerFactId,
    outbox: Option<MemberOutboxContextId>,
}

impl<'a> FounderHistory<'a> {
    pub(super) fn new(
        store: &'a ServerStore,
        verifier: &'a V2GeneralPrefixVerifier,
        owner: V2TextOwnerRef<'a>,
        channel: u128,
        admission: OwnerFactId,
        outbox: Option<MemberOutboxContextId>,
    ) -> Result<Self, V2MemberSaveRefusal> {
        let genesis = owner.source().data();
        let context = genesis.context.spec_data();
        let profile = verifier.replay_data().profile();
        if profile.id() != genesis.profile.id() || profile.spec().group != context.group {
            return Err(V2MemberSaveRefusal::Stale);
        }
        let group = context
            .group
            .try_into()
            .map_err(|_| V2MemberSaveRefusal::Failed)?;
        let scope = SharedHistoryScope {
            vault: context.vault,
            group,
            profile: profile.id(),
            tenure: profile.tenure(),
            creation: profile.spec().creation,
            channel,
        };
        Ok(Self {
            store,
            verifier,
            owner,
            scope,
            admission,
            outbox,
        })
    }

    pub(super) fn page(
        &self,
        query: SharedHistoryActionQuery,
    ) -> Result<Option<SharedHistoryActionPage>, V2MemberSaveRefusal> {
        if !self
            .store
            .has_mounted_shared_history()
            .map_err(|_| V2MemberSaveRefusal::Busy)?
        {
            return Ok(None);
        }
        self.store
            .shared_history_action_page(self.scope, query)
            .map(Some)
            .map_err(|_| V2MemberSaveRefusal::Failed)
    }

    pub(super) fn save(
        &self,
        original: &SelectedV2MemberEnvelope,
        ledger: &ChatLedger,
        rng: &mut impl CryptoRngCore,
        require_current: impl FnMut() -> Result<(), AppError>,
    ) -> Result<Option<bool>, AppError> {
        archive_founder_member(
            (self.store, self.scope),
            original,
            (ledger, self.verifier),
            rng,
            |dependency, rng| {
                let mount = self
                    .store
                    .member_outbox_mount()
                    .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
                let key =
                    member_outbox_key(self.store).map_err(|_| V2MemberSaveRefusal::Unavailable)?;
                self.saved(&mount, &key, dependency, rng)
            },
            require_current,
        )
    }

    pub(super) fn message(
        &self,
        domain: &V2TextProjectionDomain,
        retired: &[catcoms_replication::chat::V2FrozenAdmission],
        create: ActionId,
        signal: RequestCancellation,
        rng: &mut impl CryptoRngCore,
        pool: &CriticalCatalogReadPool,
    ) -> Result<Option<FounderArchivedMessage>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !self
            .store
            .has_mounted_shared_history()
            .map_err(|_| Refusal::Busy)?
        {
            return Ok(None);
        }
        let fee = pool.try_reserve(LEDGER_FEE).map_err(|_| Refusal::Busy)?;
        let realm =
            Realm::new(&self.scope.group, self.scope.channel).map_err(|_| Refusal::Failed)?;
        let mut replay = SharedHistoryReplayCredit::default();
        let mut ledger =
            ChatLedger::new(realm, ARCHIVED_LEDGER_LIMITS).map_err(|_| Refusal::Failed)?;
        let mut view = domain
            .begin_archive_channel_view(&mut ledger)
            .map_err(|_| Refusal::Unavailable)?;
        if !view.try_ready() {
            return Err(Refusal::Busy);
        }
        let mount = self
            .store
            .member_outbox_mount()
            .map_err(|_| Refusal::Unavailable)?;
        let key = member_outbox_key(self.store).map_err(|_| Refusal::Unavailable)?;
        let selected = SharedHistoryReplay {
            store: self.store,
            scope: self.scope,
            verifier: self.verifier,
            caller: &signal,
        }
        .replay_message(&mut ledger, &view, create, retired, &mut replay, |action| {
            self.saved(&mount, &key, action, rng)
        })?;
        let Some(selected) = selected else {
            return Ok(None);
        };
        let projection_fee = pool
            .try_reserve(2 * MAX_ACTION_BYTES + 32 * 1024)
            .map_err(|_| Refusal::Busy)?;
        let projection = ledger
            .guarded_projection(create, ARCHIVED_PROJECTION_WORK)
            .map_err(|_| Refusal::Failed)?;
        Ok(Some(FounderArchivedMessage {
            projection,
            signal,
            lease: Box::new(SharedHistoryProjectionLease {
                selected,
                _view: view,
                _ledger: ledger,
                review: None,
                _invocation: None,
                _fee: fee,
                _replay: replay,
            }),
            fee: projection_fee,
        }))
    }

    pub(super) fn saved(
        &self,
        mount: &MemberOutboxMount,
        key: &[u8; 32],
        action: ActionId,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Option<MemberSavedOriginal>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let context = super::super::super::member_save::member_outbox_context(
            &self.owner,
            mount.capability().current_layout_data(),
        )
        .map_err(|_| Refusal::Unavailable)?;
        if self.outbox.is_some_and(|id| id != context.id()) {
            return Err(Refusal::Stale);
        }
        let saved = mount
            .lookup_existing_action(self.store, context, key, action, rng)
            .map_err(|_| Refusal::Failed)?;
        if saved.as_ref().is_some_and(|saved| {
            let realm = &saved.action().spec().realm;
            saved.receipt().admission != self.admission
                || realm.channel() != self.scope.channel
                || realm.group() != self.scope.group.as_slice()
        }) {
            return Err(Refusal::Stale);
        }
        Ok(saved)
    }
}
