//! Original founder custody for a connected, explicitly approved holder.
//! A network await retains only bounded request data, payment and negatives.
pub(in crate::store) use super::review_decision::{
    run_founder_review_transaction, FounderReviewSign, WarmFounderReviewKeys,
};
use super::*;
use crate::store::critical_namespaces::CatalogReadReservation;
use crate::store::persistence::native_registry::network::NetworkDependency;
use crate::store::persistence::v2_member_outbox::MemberSavedOriginal;
use crate::store::persistence::v2_member_transport::{
    replay_local_member_dependencies, HISTORY_DEPENDENCY_READS,
};
use crate::store::persistence::v2_shared_history::{
    SharedHistoryScope, MAX_SHARED_HISTORY_RECORDS,
};
use crate::store::{ServerStore, V2MemberSaveRefusal};
use catcoms_net::{
    NativeChatMemberDependencies, NativeChatMemberDependencyExchange,
    NativeChatMemberDependencyObservation, NativeChatMemberInventoryExchange,
};
use catcoms_replication::chat::{
    owner_facts::OwnerFactId, ActionId, ChatLedger, LedgerLimits, ProjectionGroupView,
    ProjectionView, Realm, SignedAction, V2FrozenAdmission, V2GeneralPrefixVerifier,
    V2RemovedCreateEvidence, MAX_ACTION_BYTES, MAX_PARENTS,
};
use catcoms_rt::{
    ConnectionLease, NativeChatSourceObservation, CHAT_V2_DEPENDENCY_RESPONSE_MAX_BYTES,
};
use catcoms_sync::owner_v2::{
    PreparedV2RemovedReviewRequest, V2ArchiveHistoryCursor, V2FounderMemberDependencyAccess,
    V2MemberPublicationObservation, V2RemovedReviewOriginal, V2RemovedReviewPage,
    V2RemovedReviewQuery, V2RemovedReviewTarget,
};
use catcoms_sync::{V2MemberContactData, V2_MEMBER_CONTACT_BYTES};
use std::{
    collections::BTreeSet,
    sync::{Mutex, Weak},
};

catcoms_rt::redacted_debug! {
    Holder;
    ExchangeAuthority;
    ReviewPayment;
    NativeTextFounderReviewPage;
    NativeTextFounderReviewSweep;
    NativeTextFounderReviewExchange;
    NativeTextFounderReviewReply;
    NativeTextFounderReviewResult;
}

pub(in crate::store) const WIRE_FEE: usize = 6 * CHAT_V2_DEPENDENCY_RESPONSE_MAX_BYTES + 16 * 1024;
pub(in crate::store) const SWEEP_FEE: usize = MAX_SHARED_HISTORY_RECORDS * 128 + 16 * 1024;
pub(in crate::store) const REPLY_FEE: usize = 16 * 1024;
const ANCESTRY_BYTES: usize = HISTORY_DEPENDENCY_READS * MAX_ACTION_BYTES;
const ANCESTRY_WORK: usize = 4 * HISTORY_DEPENDENCY_READS + MAX_PARENTS;
pub(in crate::store) const VERIFY_FEE: usize = 2 * ANCESTRY_BYTES
    + HISTORY_DEPENDENCY_READS * (MAX_PARENTS + 2) * 256
    + 2 * V2GeneralPrefixVerifier::removed_create_review_temporary_layout_bytes()
    + 2 * MAX_ACTION_BYTES;

struct ReviewPayment {
    _fee: CatalogReadReservation,
}

struct Holder {
    peer: PeerId,
    admission: OwnerFactId,
    contact: [u8; V2_MEMBER_CONTACT_BYTES],
    owner: ReviewOrigin,
    connection: ConnectionLease,
    route: NativeChatMemberDependencyObservation,
    approval: RequestCancellation,
    _fee: CatalogReadReservation,
}

// A scheduled sweep contains only the exact issuer's non-owning identity.
// Neither family can resurrect or substitute a founder after remount.
enum ReviewOrigin {
    Recovered(Weak<NetworkDependency>),
    Warm(NativeChatSourceObservation),
}

pub(in crate::store) enum FounderReviewOrigin<'a> {
    Recovered(&'a Arc<NetworkDependency>),
    Warm(&'a NativeChatSourceObservation),
}

impl FounderReviewOrigin<'_> {
    fn identity(&self) -> ReviewOrigin {
        match self {
            Self::Recovered(owner) => ReviewOrigin::Recovered(Arc::downgrade(owner)),
            Self::Warm(owner) => ReviewOrigin::Warm((*owner).clone()),
        }
    }

    fn matches(&self, identity: &ReviewOrigin) -> bool {
        match (self, identity) {
            (Self::Recovered(owner), ReviewOrigin::Recovered(identity)) => {
                identity.ptr_eq(&Arc::downgrade(owner))
            }
            (Self::Warm(owner), ReviewOrigin::Warm(identity)) => owner.same_original(identity),
            _ => false,
        }
    }
}

struct ExchangeAuthority {
    caller: RequestCancellation,
    holder: Arc<Holder>,
}

#[async_trait::async_trait]
impl RequestAuthority for ExchangeAuthority {
    fn authorized(&self) -> bool {
        !self.caller.is_cancelled() && self.holder.authorized()
    }

    async fn revoked(&self) {
        let mut caller = self.caller.clone();
        tokio::select! {
            _ = caller.cancelled() => {}
            _ = self.holder.revoked() => {}
        }
    }
}

#[async_trait::async_trait]
impl RequestAuthority for Holder {
    fn authorized(&self) -> bool {
        !self.approval.is_cancelled() && self.route.is_current_cached()
    }

    async fn revoked(&self) {
        let mut approval = self.approval.clone();
        tokio::select! {
            _ = approval.cancelled() => {}
            _ = self.route.revoked() => {}
        }
    }
}

/// Same approved connection, selected lost author and channel across a
/// discovery sweep. Replacing the connection requires a fresh approval.
pub struct NativeTextFounderReviewSweep {
    counted: Mutex<BTreeSet<ActionId>>,
    holder: Arc<Holder>,
    channel: u128,
    target: V2RemovedReviewTarget,
}

impl NativeTextFounderReviewSweep {
    /// Count each verified outcome once in this sweep's native summary.
    pub fn count_result_once(&self, action: ActionId) -> Result<bool, AppError> {
        let mut counted = self.counted.try_lock().map_err(|_| refused())?;
        if counted.contains(&action) {
            return Ok(false);
        }
        if counted.len() == MAX_SHARED_HISTORY_RECORDS {
            return Err(refused());
        }
        Ok(counted.insert(action))
    }
}

enum Network {
    Inventory(NativeChatMemberInventoryExchange),
    Exact(NativeChatMemberDependencyExchange),
}

/// A paid, already sealed query ready for one connected native request.
pub struct NativeTextFounderReviewExchange {
    sweep: Arc<NativeTextFounderReviewSweep>,
    network: Network,
    request: PreparedV2RemovedReviewRequest,
    query: V2RemovedReviewQuery,
    signal: RequestCancellation,
}

pub struct NativeTextFounderReviewReply {
    sweep: Arc<NativeTextFounderReviewSweep>,
    request: PreparedV2RemovedReviewRequest,
    query: V2RemovedReviewQuery,
    wire: bytes::Bytes,
    // The original wire and decoded request drain before their payer.
    signal: RequestCancellation,
}

pub enum NativeTextFounderReviewResult {
    Page(NativeTextFounderReviewPage),
    Held(ActionId),
    AlreadyInHistory(ActionId),
}

pub struct NativeTextFounderReviewPage {
    page: V2RemovedReviewPage,
    signal: RequestCancellation,
    _fee: CatalogReadReservation,
}

impl NativeTextFounderReviewPage {
    /// Borrow this paid result only while the original route and row live.
    pub fn current_data(
        &self,
    ) -> Option<(
        &[ActionId],
        V2ArchiveHistoryCursor,
        Option<V2ArchiveHistoryCursor>,
    )> {
        (!self.signal.is_cancelled()).then(|| {
            (
                self.page.actions(),
                self.page.offer(),
                self.page.continuation(),
            )
        })
    }
}

impl NativeTextFounderReviewExchange {
    pub async fn request(self) -> Result<NativeTextFounderReviewReply, AppError> {
        let Self {
            sweep,
            network,
            request,
            query,
            signal,
        } = self;
        if !sweep.holder.route.is_current_cached() || signal.is_cancelled() {
            return Err(refused());
        }
        let wire = match network {
            Network::Inventory(route) => route.request(request.wire_data(), signal.clone()).await,
            Network::Exact(route) => route.request(request.wire_data(), signal.clone()).await,
        }
        .map_err(|_| refused())?;
        if signal.is_cancelled() || !sweep.holder.route.is_current_cached() {
            return Err(refused());
        }
        Ok(NativeTextFounderReviewReply {
            sweep,
            request,
            query,
            signal,
            wire,
        })
    }
}

pub(in crate::store) struct FounderReviewIntake<'a> {
    pub(in crate::store) origin: FounderReviewOrigin<'a>,
    pub(in crate::store) verifier: &'a V2GeneralPrefixVerifier,
    pub(in crate::store) frozen: &'a [V2FrozenAdmission],
}

pub(in crate::store) enum OpenedFounderReview {
    Page(NativeTextFounderReviewPage),
    Original(ActionId, V2RemovedReviewOriginal),
}

pub(in crate::store) enum FounderReviewCommand<'a> {
    Start(OwnerFactId, &'a [u8; V2_MEMBER_CONTACT_BYTES]),
    Prepare(Arc<NativeTextFounderReviewSweep>, V2RemovedReviewQuery),
    Receive(&'a NativeTextFounderReviewReply),
}

pub(in crate::store) enum FounderReviewOutcome {
    Sweep(Arc<NativeTextFounderReviewSweep>),
    Exchange(Box<NativeTextFounderReviewExchange>),
    Received(NativeTextFounderReviewResult),
}

impl NativeTextFounderReviewSweep {
    pub(in crate::store) fn channel_data(&self) -> u128 {
        self.channel
    }
}

impl NativeTextFounderReviewReply {
    pub(in crate::store) fn scope_data(
        &self,
    ) -> (u128, V2RemovedReviewTarget, &RequestCancellation) {
        (self.sweep.channel, self.sweep.target, &self.signal)
    }
}

impl FounderReviewIntake<'_> {
    pub(in crate::store) fn start(
        &self,
        (channel, admission, raw_contact): (u128, OwnerFactId, [u8; V2_MEMBER_CONTACT_BYTES]),
        lane: &NativeChatMemberDependencies,
        observation: V2MemberPublicationObservation,
        approval: &RequestCancellation,
        fee: CatalogReadReservation,
    ) -> Result<Arc<NativeTextFounderReviewSweep>, AppError> {
        let target = frozen_target(self.verifier, self.frozen, admission)?;
        let contact = V2MemberContactData::decode(&raw_contact)?;
        let peer = PeerId::new(*blake3::hash(contact.address_data().0).as_bytes());
        let holder_admission = contact.require_review_holder(self.verifier, peer)?;
        let route = lane.capture_inventory(peer).ok_or_else(refused)?;
        let holder = Holder {
            peer,
            admission: holder_admission,
            contact: raw_contact,
            owner: self.origin.identity(),
            connection: route.observe_connection(),
            route: route.observe_route(),
            approval: observation.constrain(approval.clone()),
            _fee: fee,
        };
        self.require_holder(&holder)?;
        Ok(Arc::new(NativeTextFounderReviewSweep {
            counted: Mutex::new(BTreeSet::new()),
            holder: Arc::new(holder),
            target,
            channel,
        }))
    }

    pub(in crate::store) fn prepare(
        &self,
        (sweep, query): (Arc<NativeTextFounderReviewSweep>, V2RemovedReviewQuery),
        lane: &NativeChatMemberDependencies,
        access: V2FounderMemberDependencyAccess<'_>,
        caller: &RequestCancellation,
        fee: CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextFounderReviewExchange, AppError> {
        self.require_sweep(&sweep)?;
        let peer = sweep.holder.peer;
        let (network, connection) = match query {
            V2RemovedReviewQuery::Page(_) => {
                let route = lane.capture_inventory(peer).ok_or_else(refused)?;
                let connection = route.observe_connection();
                (Network::Inventory(route), connection)
            }
            V2RemovedReviewQuery::Original { .. } => {
                let route = lane.capture_request(peer).ok_or_else(refused)?;
                let connection = route.observe_connection();
                (Network::Exact(route), connection)
            }
        };
        if !connection.is_same(&sweep.holder.connection) {
            return Err(refused());
        }
        let request = access.prepare_removed_review(peer, sweep.target, query, rng)?;
        let authority = Arc::new(ExchangeAuthority {
            caller: caller.clone(),
            holder: sweep.holder.clone(),
        });
        let signal = access
            .into_observation()
            .constrain(caller.clone().with_authority(authority))
            .retain_custody(Arc::new(ReviewPayment { _fee: fee }));
        if signal.is_cancelled() {
            return Err(refused());
        }
        Ok(NativeTextFounderReviewExchange {
            sweep,
            network,
            request,
            query,
            signal,
        })
    }

    pub(in crate::store) fn open(
        &self,
        reply: &NativeTextFounderReviewReply,
        access: V2FounderMemberDependencyAccess<'_>,
        fee: CatalogReadReservation,
    ) -> Result<OpenedFounderReview, AppError> {
        self.require_reply(reply)?;
        let peer = reply.sweep.holder.peer;
        let result = match reply.query {
            V2RemovedReviewQuery::Page(_) => {
                OpenedFounderReview::Page(NativeTextFounderReviewPage {
                    page: access.open_removed_review_page(peer, &reply.request, &reply.wire)?,
                    signal: reply.signal.clone(),
                    _fee: fee,
                })
            }
            V2RemovedReviewQuery::Original { action, .. } => OpenedFounderReview::Original(
                action,
                access.open_removed_review_original(peer, &reply.request, &reply.wire)?,
            ),
        };
        self.require_reply(reply)?;
        Ok(result)
    }

    pub(in crate::store) fn require_reply(
        &self,
        reply: &NativeTextFounderReviewReply,
    ) -> Result<(), AppError> {
        if reply.signal.is_cancelled() || reply.request.target() != reply.sweep.target {
            return Err(refused());
        }
        self.require_sweep(&reply.sweep)
    }

    fn require_sweep(&self, sweep: &NativeTextFounderReviewSweep) -> Result<(), AppError> {
        self.require_holder(&sweep.holder)?;
        if frozen_target(self.verifier, self.frozen, sweep.target.admission)? != sweep.target {
            return Err(refused());
        }
        Ok(())
    }

    fn require_holder(&self, holder: &Holder) -> Result<(), AppError> {
        if !holder.authorized()
            || !self.origin.matches(&holder.owner)
            || V2MemberContactData::decode(&holder.contact)?
                .require_review_holder(self.verifier, holder.peer)?
                != holder.admission
        {
            return Err(refused());
        }
        Ok(())
    }
}

impl NativeTextColdFounderSource {
    pub(in crate::store::critical_namespaces::native_registry) fn start_removed_review(
        &mut self,
        channel: u128,
        admission: OwnerFactId,
        raw_contact: [u8; V2_MEMBER_CONTACT_BYTES],
        approval: &RequestCancellation,
    ) -> Result<Arc<NativeTextFounderReviewSweep>, AppError> {
        self.restore_first_add(approval)?;
        let (mount, fee) = self.work.recovered_founder_resources(SWEEP_FEE)?;
        let governed = self.governed.as_ref().ok_or_else(refused)?;
        let (verifier, _) = governed.member_parts()?;
        let intake = FounderReviewIntake {
            origin: FounderReviewOrigin::Recovered(&self.selection.network),
            verifier,
            frozen: governed.retired_member_history()?,
        };
        let lane = self
            .tcp
            .as_ref()
            .ok_or_else(refused)?
            .capture_member_dependencies()
            .map_err(|_| refused())?;
        let sdk = self.sdk.as_ref().ok_or_else(refused)?;
        let (_, observation) = sdk.prepare_member_topic(verifier, channel, mount)?;
        let sweep = intake.start(
            (channel, admission, raw_contact),
            &lane,
            observation,
            approval,
            fee,
        )?;
        self.work
            .require_recovered_founder_source(&self.selection, &self.source)?;
        Ok(sweep)
    }

    pub(in crate::store::critical_namespaces::native_registry) fn prepare_removed_review(
        &mut self,
        sweep: Arc<NativeTextFounderReviewSweep>,
        query: V2RemovedReviewQuery,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextFounderReviewExchange, AppError> {
        self.restore_first_add(caller)?;
        let (mount, fee) = self.work.recovered_founder_resources(WIRE_FEE)?;
        let governed = self.governed.as_ref().ok_or_else(refused)?;
        let (verifier, _) = governed.member_parts()?;
        let intake = FounderReviewIntake {
            origin: FounderReviewOrigin::Recovered(&self.selection.network),
            verifier,
            frozen: governed.retired_member_history()?,
        };
        let lane = self
            .tcp
            .as_ref()
            .ok_or_else(refused)?
            .capture_member_dependencies()
            .map_err(|_| refused())?;
        let access = self
            .sdk
            .as_ref()
            .ok_or_else(refused)?
            .select_member_dependency_access(verifier, sweep.channel, self.local.0, mount)?;
        let prepared = intake.prepare((sweep, query), &lane, access, caller, fee, rng)?;
        self.work
            .require_recovered_founder_source(&self.selection, &self.source)?;
        Ok(prepared)
    }

    pub(in crate::store::critical_namespaces::native_registry) fn receive_removed_review(
        &mut self,
        reply: &NativeTextFounderReviewReply,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextFounderReviewResult, AppError> {
        self.restore_first_add(caller)?;
        let governed = self.governed.as_ref().ok_or_else(refused)?;
        let (verifier, _) = governed.member_parts()?;
        let intake = FounderReviewIntake {
            origin: FounderReviewOrigin::Recovered(&self.selection.network),
            verifier,
            frozen: governed.retired_member_history()?,
        };
        let (mount, fee) = self.work.recovered_founder_resources(REPLY_FEE)?;
        let access = self
            .sdk
            .as_ref()
            .ok_or_else(refused)?
            .select_member_dependency_access(verifier, reply.sweep.channel, self.local.0, mount)?;
        // The encrypted response retains WIRE_FEE while its opened original
        // and the independently paid local dependency ledger are in use.
        let result = match intake.open(reply, access, fee)? {
            OpenedFounderReview::Page(page) => NativeTextFounderReviewResult::Page(page),
            OpenedFounderReview::Original(action, original) => {
                let (_, _work) = self.work.recovered_founder_resources(VERIFY_FEE)?;
                self.retain_selected_review_original(
                    &reply.sweep,
                    (&reply.signal, caller),
                    &original,
                    rng,
                )?
                .map_or(
                    NativeTextFounderReviewResult::AlreadyInHistory(action),
                    |_| NativeTextFounderReviewResult::Held(action),
                )
            }
        };
        self.work
            .require_recovered_founder_source(&self.selection, &self.source)?;
        if caller.is_cancelled() || reply.signal.is_cancelled() {
            return Err(refused());
        }
        Ok(result)
    }

    fn retain_selected_review_original(
        &mut self,
        sweep: &NativeTextFounderReviewSweep,
        (request, caller): (&RequestCancellation, &RequestCancellation),
        original: &V2RemovedReviewOriginal,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<()>, AppError> {
        let scope = self.history_scope(sweep.channel)?;
        let (work, selection, source) = (&self.work, &self.selection, &self.source);
        let governed = self.governed.as_mut().ok_or_else(refused)?;
        let current = || {
            if request.is_cancelled()
                || caller.is_cancelled()
                || sweep.holder.approval.is_cancelled()
                || !sweep.holder.route.is_current_cached()
            {
                return Err(refused());
            }
            work.require_recovered_founder_source(selection, source)
        };
        work.with_recovered_founder_store(selection, source, |store| {
            governed.with_checked_member_parts(store, source, |verifier, group, frozen, require| {
                let target = sweep.target;
                let author = frozen_author(frozen, target.admission)?;
                if verifier
                    .removed_review_coordinates(author)
                    .map_err(|_| refused())?
                    != (target.removal, target.epoch)
                {
                    return Err(refused());
                }
                let exact = original.original_data();
                let mut require_current = || {
                    current()?;
                    require()
                };
                let ancestry = recovered_review_ancestry(
                    (store, scope, selection, source),
                    (verifier, group, frozen),
                    (author, exact),
                    &mut require_current,
                    rng,
                )?;
                let derive = |bytes: &[u8]| ancestry.evidence(verifier, author, bytes);
                let evidence = derive(exact)?;
                let retained = store.retain_removed_create(
                    scope,
                    (verifier, &evidence, exact),
                    require_current,
                    derive,
                    rng,
                )?;
                Ok(retained.map(|_| ()))
            })
        })
    }
}

pub(in crate::store) fn frozen_author(
    frozen: &[V2FrozenAdmission],
    admission: OwnerFactId,
) -> Result<&V2FrozenAdmission, AppError> {
    frozen
        .iter()
        .find(|author| author.admission_id_data() == admission)
        .ok_or_else(refused)
}

pub(in crate::store) struct ReviewAncestry<'a> {
    view: ProjectionView,
    ledger: ChatLedger,
    retired: Vec<&'a V2FrozenAdmission>,
}

impl ReviewAncestry<'_> {
    pub(in crate::store) fn evidence(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        author: &V2FrozenAdmission,
        bytes: &[u8],
    ) -> Result<V2RemovedCreateEvidence, AppError> {
        verifier
            .verify_removed_create_for_review(
                bytes,
                author,
                &self.ledger,
                &self.view,
                &self.retired,
            )
            .map_err(|_| refused())
    }
}

pub(super) fn recovered_review_ancestry<'a>(
    (store, scope, selection, source): (
        &ServerStore,
        SharedHistoryScope,
        &ColdFounderSelection,
        &RetainedBootstrapSourceData,
    ),
    (verifier, group, frozen): (
        &V2GeneralPrefixVerifier,
        &ProjectionGroupView,
        &'a [V2FrozenAdmission],
    ),
    (author, exact): (&V2FrozenAdmission, &[u8]),
    require_current: &mut dyn FnMut() -> Result<(), AppError>,
    rng: &mut dyn CryptoRngCore,
) -> Result<ReviewAncestry<'a>, AppError> {
    review_ancestry(
        (store, scope),
        (verifier, frozen),
        (author, exact),
        |ledger| {
            ledger
                .begin_projection_view_in(group)
                .map_err(|_| refused())
        },
        |id| {
            // The original outbox may not have been opened since restart.
            // Reuse the ordinary exact read without creating an outbox.
            history::saved_dependency(
                store,
                history::outbox_context(selection, source),
                source,
                scope,
                id,
                rng,
            )
        },
        require_current,
    )
}

/// Resolve authenticated local dependencies before taking the strict ID8
/// absence guard. A missing dependency can be held for review, while
/// approval waits for a verified parent in this original member domain.
pub(in crate::store) fn review_ancestry<'a>(
    (store, scope): (&ServerStore, SharedHistoryScope),
    (verifier, frozen): (&V2GeneralPrefixVerifier, &'a [V2FrozenAdmission]),
    (author, exact): (&V2FrozenAdmission, &[u8]),
    begin_view: impl FnOnce(&mut ChatLedger) -> Result<ProjectionView, AppError>,
    saved_dependency: impl FnMut(ActionId) -> Result<Option<MemberSavedOriginal>, V2MemberSaveRefusal>,
    require_current: &mut dyn FnMut() -> Result<(), AppError>,
) -> Result<ReviewAncestry<'a>, AppError> {
    let realm = Realm::new(&scope.group, scope.channel).map_err(|_| refused())?;
    let mut ledger = ChatLedger::new(
        realm,
        LedgerLimits {
            max_actions: HISTORY_DEPENDENCY_READS,
            max_action_bytes: ANCESTRY_BYTES,
        },
    )
    .map_err(|_| refused())?;
    let view = begin_view(&mut ledger)?;
    let retired = frozen.iter().collect::<Vec<_>>();
    let action = SignedAction::decode(exact).map_err(|_| refused())?;
    verifier
        .require_removed_review_create(author, &action)
        .map_err(|_| refused())?;
    let mut roots = action.spec().parents.clone();
    if let catcoms_replication::chat::Body::Create {
        reply_to: Some(parent),
        ..
    } = action.spec().body
    {
        if !roots.contains(&parent) {
            roots.push(parent);
        }
    }
    if !roots.is_empty() {
        replay_local_member_dependencies(
            &roots,
            (verifier, &retired, &mut ledger, &view),
            ANCESTRY_WORK,
            (store, scope),
            saved_dependency,
            || require_current().map_err(|_| crate::store::V2MemberSaveRefusal::Stale),
        )
        .map_err(|_| refused())?;
    }
    Ok(ReviewAncestry {
        view,
        ledger,
        retired,
    })
}

fn frozen_target(
    verifier: &V2GeneralPrefixVerifier,
    frozen: &[V2FrozenAdmission],
    admission: OwnerFactId,
) -> Result<V2RemovedReviewTarget, AppError> {
    let (removal, epoch) = verifier
        .removed_review_coordinates(frozen_author(frozen, admission)?)
        .map_err(|_| refused())?;
    Ok(V2RemovedReviewTarget {
        admission,
        removal,
        epoch,
    })
}
