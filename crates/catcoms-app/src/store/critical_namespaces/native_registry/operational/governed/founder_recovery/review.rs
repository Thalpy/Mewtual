//! Original founder custody for a connected, explicitly approved holder.
//! A network await retains only bounded request data, payment and negatives.
use super::*;
use crate::store::critical_namespaces::CatalogReadReservation;
use crate::store::persistence::native_registry::network::NetworkDependency;
use crate::store::persistence::v2_member_outbox::{member_outbox_key, MemberOutboxContextId};
use crate::store::persistence::v2_member_transport::{
    replay_local_member_dependencies, HISTORY_DEPENDENCY_READS,
};
use crate::store::persistence::v2_shared_history::{
    SharedHistoryScope, MAX_SHARED_HISTORY_RECORDS,
};
use crate::store::ServerStore;
use catcoms_net::{
    NativeChatMemberDependencyExchange, NativeChatMemberDependencyObservation,
    NativeChatMemberInventoryExchange,
};
use catcoms_replication::chat::{
    owner_facts::OwnerFactId, ActionId, ChatLedger, LedgerLimits, ProjectionGroupView,
    ProjectionView, Realm, SignedAction, V2FrozenAdmission, V2GeneralPrefixVerifier,
    V2RemovedCreateEvidence, MAX_ACTION_BYTES, MAX_PARENTS,
};
use catcoms_rt::{ConnectionLease, CHAT_V2_DEPENDENCY_RESPONSE_MAX_BYTES};
use catcoms_sync::owner_v2::{
    PreparedV2RemovedReviewRequest, V2ArchiveHistoryCursor, V2RemovedReviewOriginal,
    V2RemovedReviewPage, V2RemovedReviewQuery, V2RemovedReviewTarget,
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

const WIRE_FEE: usize = 6 * CHAT_V2_DEPENDENCY_RESPONSE_MAX_BYTES + 16 * 1024;
const SWEEP_FEE: usize = MAX_SHARED_HISTORY_RECORDS * 128 + 16 * 1024;
const ANCESTRY_BYTES: usize = HISTORY_DEPENDENCY_READS * MAX_ACTION_BYTES;
const ANCESTRY_WORK: usize = 4 * HISTORY_DEPENDENCY_READS + MAX_PARENTS;
pub(super) const VERIFY_FEE: usize = 2 * ANCESTRY_BYTES
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
    owner: Weak<NetworkDependency>,
    connection: ConnectionLease,
    route: NativeChatMemberDependencyObservation,
    approval: RequestCancellation,
    _fee: CatalogReadReservation,
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
        let target = frozen_target(verifier, governed.retired_member_history()?, admission)?;
        let contact = V2MemberContactData::decode(&raw_contact)?;
        let peer = PeerId::new(*blake3::hash(contact.address_data().0).as_bytes());
        let holder_admission = contact.require_review_holder(verifier, peer)?;
        let lane = self
            .tcp
            .as_ref()
            .ok_or_else(refused)?
            .capture_member_dependencies()
            .map_err(|_| refused())?;
        let route = lane.capture_inventory(peer).ok_or_else(refused)?;
        let sdk = self.sdk.as_ref().ok_or_else(refused)?;
        let (_, observation) = sdk.prepare_member_topic(verifier, channel, mount)?;
        let holder = Holder {
            peer,
            admission: holder_admission,
            contact: raw_contact,
            owner: Arc::downgrade(&self.selection.network),
            connection: route.observe_connection(),
            route: route.observe_route(),
            approval: observation.constrain(approval.clone()),
            _fee: fee,
        };
        self.require_review_holder(&holder, verifier)?;
        Ok(Arc::new(NativeTextFounderReviewSweep {
            counted: Mutex::new(BTreeSet::new()),
            holder: Arc::new(holder),
            target,
            channel,
        }))
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
        self.require_review_holder(&sweep.holder, verifier)?;
        if frozen_target(
            verifier,
            governed.retired_member_history()?,
            sweep.target.admission,
        )? != sweep.target
        {
            return Err(refused());
        }
        let lane = self
            .tcp
            .as_ref()
            .ok_or_else(refused)?
            .capture_member_dependencies()
            .map_err(|_| refused())?;
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
        let sdk = self.sdk.as_ref().ok_or_else(refused)?;
        let access = sdk.select_member_dependency_access(
            verifier,
            sweep.channel,
            self.local.0,
            mount.clone(),
        )?;
        let request = access.prepare_removed_review(peer, sweep.target, query, rng)?;
        let (_, observation) = sdk.prepare_member_topic(verifier, sweep.channel, mount)?;
        let authority = Arc::new(ExchangeAuthority {
            caller: caller.clone(),
            holder: sweep.holder.clone(),
        });
        let signal = observation
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

    pub(in crate::store::critical_namespaces::native_registry) fn receive_removed_review(
        &mut self,
        reply: NativeTextFounderReviewReply,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextFounderReviewResult, AppError> {
        self.restore_first_add(caller)?;
        let NativeTextFounderReviewReply {
            sweep,
            request,
            query,
            signal,
            wire,
        } = reply;
        let governed = self.governed.as_ref().ok_or_else(refused)?;
        let (verifier, _) = governed.member_parts()?;
        self.require_review_holder(&sweep.holder, verifier)?;
        if signal.is_cancelled()
            || request.target() != sweep.target
            || frozen_target(
                verifier,
                governed.retired_member_history()?,
                sweep.target.admission,
            )? != sweep.target
        {
            return Err(refused());
        }
        let (mount, fee) = self.work.recovered_founder_resources(16 * 1024)?;
        let sdk = self.sdk.as_ref().ok_or_else(refused)?;
        let access =
            sdk.select_member_dependency_access(verifier, sweep.channel, self.local.0, mount)?;
        let result = match query {
            V2RemovedReviewQuery::Page(_) => {
                NativeTextFounderReviewResult::Page(NativeTextFounderReviewPage {
                    page: access.open_removed_review_page(sweep.holder.peer, &request, &wire)?,
                    signal: signal.clone(),
                    _fee: fee,
                })
            }
            V2RemovedReviewQuery::Original { action, .. } => {
                let (_, _work) = self.work.recovered_founder_resources(VERIFY_FEE)?;
                let original =
                    access.open_removed_review_original(sweep.holder.peer, &request, &wire)?;
                self.retain_selected_review_original(&sweep, (&signal, caller), &original, rng)?
                    .map_or(
                        NativeTextFounderReviewResult::AlreadyInHistory(action),
                        |_| NativeTextFounderReviewResult::Held(action),
                    )
            }
        };
        if signal.is_cancelled() || caller.is_cancelled() || !sweep.holder.route.is_current_cached()
        {
            return Err(refused());
        }
        Ok(result)
    }

    fn require_review_holder(
        &self,
        holder: &Holder,
        verifier: &V2GeneralPrefixVerifier,
    ) -> Result<(), AppError> {
        if holder.approval.is_cancelled()
            || !holder.route.is_current_cached()
            || !holder
                .owner
                .ptr_eq(&Arc::downgrade(&self.selection.network))
            || V2MemberContactData::decode(&holder.contact)?
                .require_review_holder(verifier, holder.peer)?
                != holder.admission
        {
            return Err(refused());
        }
        self.work
            .require_recovered_founder_source(&self.selection, &self.source)
    }

    fn retain_selected_review_original(
        &mut self,
        sweep: &NativeTextFounderReviewSweep,
        (request, caller): (&RequestCancellation, &RequestCancellation),
        original: &V2RemovedReviewOriginal,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<()>, AppError> {
        let scope = self.history_scope(sweep.channel)?;
        let outbox_context = self.outbox_context()?.id();
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
                let ancestry = review_ancestry(
                    (store, scope, outbox_context),
                    (verifier, group, frozen),
                    (author, exact),
                    &mut require_current,
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

pub(super) fn frozen_author(
    frozen: &[V2FrozenAdmission],
    admission: OwnerFactId,
) -> Result<&V2FrozenAdmission, AppError> {
    frozen
        .iter()
        .find(|author| author.admission_id_data() == admission)
        .ok_or_else(refused)
}

pub(super) struct ReviewAncestry<'a> {
    view: ProjectionView,
    ledger: ChatLedger,
    retired: Vec<&'a V2FrozenAdmission>,
}

impl ReviewAncestry<'_> {
    pub(super) fn evidence(
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

/// Resolve authenticated local dependencies before taking the strict ID8
/// absence guard. This path can retain a reply with missing ancestry for
/// review. The actual Approve body refuses until the parent is verified.
pub(super) fn review_ancestry<'a>(
    (store, scope, outbox_context): (&ServerStore, SharedHistoryScope, MemberOutboxContextId),
    (verifier, group, frozen): (
        &V2GeneralPrefixVerifier,
        &ProjectionGroupView,
        &'a [V2FrozenAdmission],
    ),
    (author, exact): (&V2FrozenAdmission, &[u8]),
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
    let view = ledger
        .begin_projection_view_in(group)
        .map_err(|_| refused())?;
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
            |id| {
                use crate::store::V2MemberSaveRefusal as Refusal;
                let Some(outbox) = store
                    .installed_member_outbox_mount()
                    .map_err(|_| Refusal::Busy)?
                else {
                    return Ok(None);
                };
                let key = member_outbox_key(store).map_err(|_| Refusal::Unavailable)?;
                outbox
                    .read_selected_history_action(store, outbox_context, &key, id)
                    .map_err(|_| Refusal::Busy)
            },
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
