//! Internal finite operations on the original selected founder member source.
use super::v2_member_outbox::{member_outbox_key, MemberDebtBatch, MemberSavedOriginal};
use super::v2_shared_history::{
    PreparedSharedHistoryRecord, SharedHistoryActionPage, SharedHistoryActionQuery,
    SharedHistoryProjectionLease, SharedHistoryScope, SHARED_HISTORY_RECORD_FEE,
};
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::critical_namespaces::refused;
use crate::store::{NativeTextJoinedMemberProjection, ServerStore, V2MemberSaveRefusal as Refusal};
use crate::AppError;
use catcoms_net::{
    NativeChatMemberDependencyExchange, NativeChatMemberDependencyIncoming,
    NativeChatMemberDependencyObservation, NativeChatMemberFrame,
};
use catcoms_replication::chat::{
    ActionId, ChatLedger, GuardedMessageProjection, ProjectionView, V2GeneralPrefixVerifier,
    V2OrdinaryReceiveStatus, MAX_PARENTS,
};
use catcoms_rt::{CryptoRngCore, RequestCancellation, Topic, TransportError};
use catcoms_sync::owner_v2::{
    PreparedV2MemberDependencyRequest, PreparedV2MemberSubmission, SelectedV2MemberEnvelope,
    V2FounderMemberDependencyAccess, V2MemberDependencyRoute, V2MemberPublicationObservation,
    MAX_V2_DEPENDENCY_REPLY_BYTES, MAX_V2_MEMBER_TRANSPORT_BYTES,
};
use std::sync::Arc;

pub(in crate::store) const FOUNDER_RECEIVE_ACTIONS: usize = 64;
pub(in crate::store) const FOUNDER_RECEIVE_BYTES: usize = 1024 * 1024;
pub(in crate::store) const FOUNDER_RECEIVE_WORK: usize = 4 * FOUNDER_RECEIVE_ACTIONS + MAX_PARENTS;
pub(in crate::store) const FOUNDER_RECEIVE_LEDGER_FEE: usize = 2 * FOUNDER_RECEIVE_BYTES
    + FOUNDER_RECEIVE_ACTIONS * (MAX_PARENTS + 2) * 256
    + V2GeneralPrefixVerifier::ordinary_authorization_temporary_layout_bytes()
    + 16 * 1024;
const LOCAL_DEPENDENCY_READS: usize = 8;
pub(in crate::store) const FOUNDER_DEPENDENCY_REPLY_FEE: usize =
    4 * MAX_V2_MEMBER_TRANSPORT_BYTES + 5 * MAX_V2_DEPENDENCY_REPLY_BYTES + 16 * 1024;

pub(in crate::store) struct FounderDependencyReply {
    pub(in crate::store) wire: bytes::Bytes,
    pub(in crate::store) observation: V2MemberPublicationObservation,
    pub(in crate::store) fee: CatalogReadReservation,
}

pub struct NativeTextFounderDependencyExchange {
    exchange: NativeChatMemberDependencyExchange,
    route: V2MemberDependencyRoute,
    request: PreparedV2MemberDependencyRequest,
    session: Arc<()>,
    signal: RequestCancellation,
}

pub struct NativeTextFounderDependencyResponse {
    route: V2MemberDependencyRoute,
    request: PreparedV2MemberDependencyRequest,
    session: Arc<()>,
    carrier: NativeChatMemberDependencyObservation,
    signal: RequestCancellation,
    wire: bytes::Bytes,
}

struct DependencyPayment {
    _fee: CatalogReadReservation,
}

impl std::fmt::Debug for DependencyPayment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DependencyPayment").finish_non_exhaustive()
    }
}

impl NativeTextFounderDependencyExchange {
    pub async fn receive(self) -> Result<NativeTextFounderDependencyResponse, TransportError> {
        let carrier = self.exchange.observe_route();
        let wire = self
            .exchange
            .request(self.request.wire_data(), self.signal.clone())
            .await?;
        if self.signal.is_cancelled() || !carrier.is_current_cached() {
            return Err(TransportError::Cancelled);
        }
        Ok(NativeTextFounderDependencyResponse {
            route: self.route,
            request: self.request,
            session: self.session,
            carrier,
            signal: self.signal,
            wire,
        })
    }
}

impl NativeTextFounderDependencyResponse {
    pub(in crate::store) fn require_current(
        &self,
        session: &FounderReceiveSession,
    ) -> Result<(), Refusal> {
        if !Arc::ptr_eq(&self.session, &session.identity)
            || self.signal.is_cancelled()
            || session.signal.is_cancelled()
            || !self.carrier.is_current_cached()
        {
            return Err(Refusal::Stale);
        }
        Ok(())
    }

    pub(in crate::store) fn open(
        &self,
        access: V2FounderMemberDependencyAccess<'_>,
        session: &FounderReceiveSession,
    ) -> Result<Option<SelectedV2MemberEnvelope>, Refusal> {
        self.require_current(session)?;
        let opened = access
            .open_reply(self.route, &self.request, &self.wire)
            .map_err(|_| Refusal::Failed)?;
        self.require_current(session)?;
        Ok(opened)
    }
}

pub(in crate::store) fn prepare_founder_dependency_exchange(
    access: V2FounderMemberDependencyAccess<'_>,
    exchange: NativeChatMemberDependencyExchange,
    session: &FounderReceiveSession,
    original: &SelectedV2MemberEnvelope,
    action: ActionId,
    fee: CatalogReadReservation,
    rng: &mut impl CryptoRngCore,
) -> Result<NativeTextFounderDependencyExchange, Refusal> {
    let carrier = exchange.observe_route();
    if session.signal.is_cancelled() || !carrier.is_current_cached() {
        return Err(Refusal::Stale);
    }
    let (requester, provider) = exchange.route_data();
    let route = V2MemberDependencyRoute {
        requester,
        provider,
    };
    let request = access
        .prepare_request(original, route, action, rng)
        .map_err(|_| Refusal::Failed)?;
    let signal = access
        .into_observation()
        .constrain(session.signal.clone())
        .retain_custody(Arc::new(DependencyPayment { _fee: fee }));
    if signal.is_cancelled() || !carrier.is_current_cached() {
        return Err(Refusal::Stale);
    }
    Ok(NativeTextFounderDependencyExchange {
        exchange,
        route,
        request,
        session: session.identity.clone(),
        signal,
    })
}

pub(in crate::store) fn prepare_founder_dependency_reply<R: CryptoRngCore>(
    provider: V2FounderMemberDependencyAccess<'_>,
    incoming: &NativeChatMemberDependencyIncoming,
    fee: CatalogReadReservation,
    caller: &RequestCancellation,
    rng: &mut R,
    saved_dependency: impl FnOnce(ActionId, &mut R) -> Result<Option<MemberSavedOriginal>, Refusal>,
) -> Result<FounderDependencyReply, Refusal> {
    if caller.is_cancelled() {
        return Err(Refusal::Cancelled);
    }
    let (requester, wire) = incoming.current_data().ok_or(Refusal::Cancelled)?;
    let route = provider.route_data(requester);
    let request = provider
        .open_request(route, wire)
        .map_err(|_| Refusal::Failed)?;
    if caller.is_cancelled() || incoming.current_data().is_none() {
        return Err(Refusal::Cancelled);
    }
    let saved = saved_dependency(request.action_id_data(), rng)?;
    if caller.is_cancelled() || incoming.current_data().is_none() {
        return Err(Refusal::Cancelled);
    }
    let wire = provider
        .seal_reply(
            route,
            &request,
            saved
                .as_ref()
                .map(|entry| (entry.receipt().admission, entry.original_bytes())),
            rng,
        )
        .map_err(|_| Refusal::Failed)?;
    Ok(FounderDependencyReply {
        wire: bytes::Bytes::from(wire),
        observation: provider.into_observation(),
        fee,
    })
}

/// Project a live envelope, then retry with its bounded locally saved causal
/// slice. The storage owner chooses which originals can be lent.
pub(in crate::store) fn project_founder_member(
    original: &SelectedV2MemberEnvelope,
    verifier: &V2GeneralPrefixVerifier,
    ledger: &mut ChatLedger,
    view: &ProjectionView,
    archive: (&ServerStore, SharedHistoryScope),
    mut saved_dependency: impl FnMut(ActionId) -> Result<Option<MemberSavedOriginal>, Refusal>,
    mut require_current: impl FnMut() -> Result<(), Refusal>,
) -> Result<V2OrdinaryReceiveStatus, Refusal> {
    require_current()?;
    let mut status = original
        .project_from_ledger(verifier, ledger, view, FOUNDER_RECEIVE_WORK)
        .map_err(|_| Refusal::Failed)?;
    if let V2OrdinaryReceiveStatus::WaitingForDependencies(missing) = &status {
        if missing.len() > 4 {
            require_current()?;
            return Ok(status);
        }
        replay_local_member_dependencies(
            missing,
            (verifier, ledger, view),
            FOUNDER_RECEIVE_WORK,
            archive,
            &mut saved_dependency,
            &mut require_current,
        )?;
        status = original
            .project_from_ledger(verifier, ledger, view, FOUNDER_RECEIVE_WORK)
            .map_err(|_| Refusal::Failed)?;
    }
    require_current()?;
    Ok(status)
}

/// Restore historical semantics without issuing live projection tokens. The
/// result describes the first requested original, when locally available.
pub(in crate::store) fn replay_local_member_dependencies(
    roots: &[ActionId],
    (verifier, ledger, view): (&V2GeneralPrefixVerifier, &mut ChatLedger, &ProjectionView),
    work: usize,
    (store, scope): (&ServerStore, SharedHistoryScope),
    mut saved_dependency: impl FnMut(ActionId) -> Result<Option<MemberSavedOriginal>, Refusal>,
    mut require_current: impl FnMut() -> Result<(), Refusal>,
) -> Result<Option<V2OrdinaryReceiveStatus>, Refusal> {
    if roots.len() > LOCAL_DEPENDENCY_READS {
        return Err(Refusal::Failed);
    }
    let mut pending = Vec::with_capacity(LOCAL_DEPENDENCY_READS);
    pending.extend(roots.iter().rev().map(|id| (*id, false)));
    let mut remaining = LOCAL_DEPENDENCY_READS;
    let mut first = None;
    while let Some((id, retry)) = pending.pop() {
        require_current()?;
        remaining = remaining.checked_sub(1).ok_or(Refusal::Failed)?;
        let own = saved_dependency(id)?;
        let archived = if own.is_none()
            && store
                .has_mounted_shared_history()
                .map_err(|_| Refusal::Busy)?
        {
            store
                .read_shared_history_record(scope, id)
                .map_err(|_| Refusal::Failed)?
        } else {
            None
        };
        require_current()?;
        let Some((action, admission)) = own
            .as_ref()
            .map(|saved| (saved.action(), saved.receipt().admission))
            .or_else(|| {
                archived
                    .as_ref()
                    .map(|saved| (saved.original.action(), saved.admission))
            })
        else {
            continue;
        };
        if action.id() != id || &action.spec().realm != ledger.realm() {
            return Err(Refusal::Stale);
        }
        let admission = verifier.admission(admission).map_err(|_| Refusal::Stale)?;
        let status = verifier
            .replay_ordinary_history_original(ledger, view, action, &admission, &[], work)
            .map_err(|_| Refusal::Failed)?;
        if let V2OrdinaryReceiveStatus::WaitingForDependencies(missing) = &status {
            // Keep enough reads to finish roots and retries already queued.
            if !retry && pending.len() + missing.len() < remaining {
                pending.push((id, true));
                pending.extend(missing.iter().rev().map(|id| (*id, false)));
            }
        }
        if roots.first() == Some(&id) {
            first = Some(status);
        }
    }
    require_current()?;
    Ok(first)
}

pub(in crate::store) fn archive_founder_member(
    store: &ServerStore,
    scope: SharedHistoryScope,
    original: &SelectedV2MemberEnvelope,
    rng: &mut impl CryptoRngCore,
    require_current: impl FnOnce() -> Result<(), AppError>,
) -> Result<Option<bool>, AppError> {
    if !store.has_mounted_shared_history()? {
        return Ok(None);
    }
    let _fee = store
        .archive_bootstrap_policy
        .as_ref()
        .ok_or_else(refused)?
        .pending
        .pool
        .try_reserve(SHARED_HISTORY_RECORD_FEE)?;
    let key = member_outbox_key(store)?;
    let record = PreparedSharedHistoryRecord::prepare(
        &scope,
        &key,
        original.admission_data(),
        original.action_data(),
        original.original_data(),
        rng,
    )?;
    let bundle = record.encode_bundle()?;
    require_current()?;
    store.save_shared_history_record(scope, record.action, record.manifest.id(), &bundle, rng)?;
    Ok(Some(true))
}

pub(in crate::store) enum FounderDebtOperation {
    Prepare,
    Checkpoint(MemberDebtBatch),
}

pub(in crate::store) enum FounderDebtOutcome {
    Idle,
    Prepared {
        submission: PreparedV2MemberSubmission,
        batch: MemberDebtBatch,
        fee: CatalogReadReservation,
    },
    Checkpointed(u64),
}

#[derive(Clone)]
pub(in crate::store) struct FounderReceiveSession {
    pub(in crate::store) identity: Arc<()>,
    pub(in crate::store) signal: RequestCancellation,
}

pub(in crate::store) enum FounderReceiveOperation<'a> {
    Subscription,
    Frame(
        &'a NativeChatMemberFrame,
        &'a FounderReceiveSession,
        Option<NativeChatMemberDependencyExchange>,
    ),
    Recover(
        &'a NativeTextFounderDependencyResponse,
        &'a FounderReceiveSession,
    ),
    Dependency(&'a NativeChatMemberDependencyIncoming),
    Read(ActionId),
    ArchivePage(SharedHistoryActionQuery),
    ArchiveMessage(ActionId),
}

/// The original founder receiver's update for a verified action.
pub struct NativeTextFounderReceived {
    pub action: ActionId,
    pub projection_target: ActionId,
    /// Whether the selected archive covered the action. No value means this
    /// vault has no mounted shared history namespace.
    pub archived: Option<bool>,
}

pub enum NativeTextFounderFrameOutcome {
    Retained(NativeTextFounderReceived),
    NeedsDependency(NativeTextFounderDependencyExchange),
    Unresolved,
}

impl NativeTextFounderFrameOutcome {
    pub(in crate::store) fn retained(self) -> Option<NativeTextFounderReceived> {
        match self {
            Self::Retained(action) => Some(action),
            Self::NeedsDependency(_) | Self::Unresolved => None,
        }
    }
}

pub(in crate::store) struct FounderArchivedMessage {
    pub(in crate::store) projection: GuardedMessageProjection,
    pub(in crate::store) signal: RequestCancellation,
    pub(in crate::store) lease: Box<SharedHistoryProjectionLease>,
    pub(in crate::store) fee: CatalogReadReservation,
}

pub(in crate::store) enum FounderReceiveOutcome {
    Dependency(FounderDependencyReply),
    Subscription(
        Topic,
        V2MemberPublicationObservation,
        CatalogReadReservation,
    ),
    Frame(NativeTextFounderFrameOutcome),
    Recovered(bool),
    Read(
        Option<(
            GuardedMessageProjection,
            RequestCancellation,
            CatalogReadReservation,
        )>,
    ),
    ArchivePage(Option<SharedHistoryActionPage>),
    ArchiveMessage(Option<FounderArchivedMessage>),
    RecoveredProjection(Option<NativeTextJoinedMemberProjection>),
}
