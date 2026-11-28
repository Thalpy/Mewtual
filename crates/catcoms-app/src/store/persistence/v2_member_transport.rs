//! Internal finite operations on the original selected founder member source.
catcoms_rt::redacted_debug! {
    NativeTextFounderDependencyExchange;
    NativeTextFounderDependencyResponse;
    NativeTextFounderReceived;
    NativeTextFounderFrameOutcome;
    NativeTextFounderAudit;
}

use super::v2_member_outbox::{member_outbox_key, MemberDebtBatch, MemberSavedOriginal};
use super::v2_shared_history::{
    PreparedSharedHistoryRecord, SelectedSharedHistoryAuditPage, SharedHistoryActionPage,
    SharedHistoryActionQuery, SharedHistoryProjectionLease, SharedHistoryScope,
    SHARED_HISTORY_RECORD_FEE,
};
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::critical_namespaces::refused;
use crate::store::{NativeTextJoinedMemberProjection, ServerStore, V2MemberSaveRefusal as Refusal};
use crate::AppError;
use catcoms_net::{
    NativeChatMemberDependencies, NativeChatMemberDependencyExchange,
    NativeChatMemberDependencyIncoming, NativeChatMemberDependencyObservation,
    NativeChatMemberFrame,
};
use catcoms_replication::chat::{
    cold_actions::AssembledActionOriginal, owner_facts::OwnerFactId, ActionId, ActionState,
    ChatLedger, GuardedMessageProjection, LedgerLimits, ProjectionView, Realm, SignedAction,
    V2FrozenAdmission, V2GeneralPrefixVerifier, V2LocalArchiveOriginal, V2OrdinaryReceiveStatus,
    MAX_PARENTS,
};
use catcoms_rt::{CryptoRngCore, RequestCancellation, Topic, TransportError};
use catcoms_sync::owner_v2::{
    PreparedV2MemberDependencyRequest, PreparedV2MemberSubmission, SelectedV2MemberEnvelope,
    V2FounderMemberDependencyAccess, V2MemberDependencyRoute, V2MemberPublicationObservation,
    MAX_V2_DEPENDENCY_REPLY_BYTES, MAX_V2_MEMBER_TRANSPORT_BYTES,
};
use std::{collections::BTreeSet, sync::Arc};

pub(in crate::store) const FOUNDER_RECEIVE_ACTIONS: usize = 64;
pub(in crate::store) const FOUNDER_RECEIVE_BYTES: usize = 1024 * 1024;
pub(in crate::store) const FOUNDER_RECEIVE_WORK: usize = 4 * FOUNDER_RECEIVE_ACTIONS + MAX_PARENTS;
pub(in crate::store) const FOUNDER_RECEIVE_LEDGER_FEE: usize = 2 * FOUNDER_RECEIVE_BYTES
    + FOUNDER_RECEIVE_ACTIONS * (MAX_PARENTS + 2) * 256
    + V2GeneralPrefixVerifier::ordinary_authorization_temporary_layout_bytes()
    + 16 * 1024;
const LOCAL_DEPENDENCY_READS: usize = 8;
pub(in crate::store) const HISTORY_DEPENDENCY_READS: usize = 128;
const HISTORY_PENDING_IDS: usize = HISTORY_DEPENDENCY_READS + MAX_PARENTS;
pub(in crate::store) const HISTORY_REPLAY_FEE: usize = HISTORY_PENDING_IDS
    * std::mem::size_of::<(ActionId, bool)>()
    + HISTORY_DEPENDENCY_READS * 96
    + 4 * 1024;
const MAX_FOUNDER_DEPENDENCY_DEPTH: u8 = 4;
const FOUNDER_RECOVERY_BREADTH: usize = 4;
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
    depth: u8,
    signal: RequestCancellation,
}

pub struct NativeTextFounderDependencyResponse {
    route: V2MemberDependencyRoute,
    request: PreparedV2MemberDependencyRequest,
    session: Arc<()>,
    depth: u8,
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
            depth: self.depth,
            carrier,
            signal: self.signal,
            wire,
        })
    }
}

impl NativeTextFounderDependencyResponse {
    pub(in crate::store) fn capture_followup(
        &self,
        lane: &NativeChatMemberDependencies,
    ) -> Option<NativeChatMemberDependencyExchange> {
        if self.depth >= MAX_FOUNDER_DEPENDENCY_DEPTH || self.signal.is_cancelled() {
            return None;
        }
        lane.capture_frame_followup(&self.carrier)
    }

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
    (session, parent): (
        &FounderReceiveSession,
        Option<&NativeTextFounderDependencyResponse>,
    ),
    original: &SelectedV2MemberEnvelope,
    action: ActionId,
    fee: CatalogReadReservation,
    rng: &mut impl CryptoRngCore,
) -> Result<NativeTextFounderDependencyExchange, Refusal> {
    let carrier = exchange.observe_route();
    if session.signal.is_cancelled() || !carrier.is_current_cached() {
        return Err(Refusal::Stale);
    }
    let (caller, depth) = if let Some(parent) = parent {
        parent.require_current(session)?;
        let depth = parent
            .depth
            .checked_add(1)
            .filter(|depth| *depth <= MAX_FOUNDER_DEPENDENCY_DEPTH)
            .ok_or(Refusal::Unavailable)?;
        (parent.signal.clone(), depth)
    } else {
        (session.signal.clone(), 1)
    };
    let (requester, provider) = exchange.route_data();
    let route = V2MemberDependencyRoute {
        requester,
        provider,
    };
    if parent.is_some_and(|parent| parent.route != route) {
        return Err(Refusal::Stale);
    }
    let request = access
        .prepare_request(original, route, action, rng)
        .map_err(|_| Refusal::Failed)?;
    let signal = access
        .into_observation()
        .constrain(caller)
        .retain_custody(Arc::new(DependencyPayment { _fee: fee }));
    if signal.is_cancelled() || !carrier.is_current_cached() {
        return Err(Refusal::Stale);
    }
    Ok(NativeTextFounderDependencyExchange {
        exchange,
        route,
        request,
        session: session.identity.clone(),
        depth,
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

/// Distinguish a replaceable receive ledger from view or storage refusals.
pub(in crate::store) fn receive_ledger_refusal(
    error: catcoms_replication::chat::ChatError,
) -> Refusal {
    match error {
        catcoms_replication::chat::ChatError::LedgerCapacity => Refusal::ReceiveCapacity,
        _ => Refusal::Failed,
    }
}

/// Reserve exact archive debt before staging in a mounted archive. A
/// legacy layout can still project directly. Missing dependencies are
/// retried with the original row's bounded locally saved causal slice.
pub(in crate::store) fn project_founder_member(
    original: &SelectedV2MemberEnvelope,
    (verifier, retired): (&V2GeneralPrefixVerifier, &[V2FrozenAdmission]),
    ledger: &mut ChatLedger,
    view: &ProjectionView,
    (store, scope, admit_mutation): (
        &ServerStore,
        SharedHistoryScope,
        &dyn FounderArchiveAdmission,
    ),
    mut saved_dependency: impl FnMut(ActionId) -> Result<Option<MemberSavedOriginal>, Refusal>,
    mut require_current: impl FnMut() -> Result<(), Refusal>,
) -> Result<V2OrdinaryReceiveStatus, Refusal> {
    require_current()?;
    let mounted = store
        .has_mounted_shared_history()
        .map_err(|_| Refusal::Busy)?;
    let action = original.action_data();
    let target = action
        .spec()
        .body
        .create()
        .or_else(|| mounted.then_some(action.id()));
    let retired = retired.iter().collect::<Vec<_>>();
    with_founder_archive_reservation(action.id(), target, admit_mutation, |observe| {
        let mut project = |ledger: &mut ChatLedger| -> Result<_, Refusal> {
            let status = original
                .apply_with_local_ancestry(
                    (verifier, &retired),
                    (ledger, view),
                    FOUNDER_RECEIVE_WORK,
                    !mounted,
                )
                .map_err(receive_ledger_refusal)?;
            observe(&status);
            Ok(status)
        };
        let mut status = project(&mut *ledger)?;
        if let V2OrdinaryReceiveStatus::WaitingForDependencies(missing) = &status {
            replay_local_member_dependencies(
                missing,
                (verifier, &retired, ledger, view),
                FOUNDER_RECEIVE_WORK,
                (store, scope),
                &mut saved_dependency,
                &mut require_current,
            )?;
            status = project(&mut *ledger)?;
        }
        require_current()?;
        Ok(status)
    })
    .map_err(|error| {
        if !mounted && error == Refusal::ReceiveCapacity {
            Refusal::Failed
        } else {
            error
        }
    })
}

fn with_founder_archive_reservation(
    action: ActionId,
    target: Option<ActionId>,
    admit: &dyn FounderArchiveAdmission,
    project: impl FnOnce(
        &mut dyn FnMut(&V2OrdinaryReceiveStatus),
    ) -> Result<V2OrdinaryReceiveStatus, Refusal>,
) -> Result<V2OrdinaryReceiveStatus, Refusal> {
    // Even a semantic import can become a dependency. Reserve before
    // making the first ledger change.
    let mut retained_valid = false;
    let newly_reserved = target
        .map(|target| admit.reserve(action, target).ok_or(Refusal::Busy))
        .transpose()?;
    let result = project(&mut |status| {
        retained_valid |= matches!(
            status,
            V2OrdinaryReceiveStatus::Retained(ActionState::Valid)
        );
    });
    if let Some(target) = target {
        if newly_reserved == Some(true) && !retained_valid {
            // Pending or invalid originals cannot reach the archive writer.
            // Keep debt for a validated exact original across cancellation.
            admit.release_unstarted(action, target);
        }
    }
    result
}

#[cfg(test)]
mod archive_reservation_tests {
    use super::*;
    use catcoms_replication::chat::ChatError;
    use std::cell::Cell;

    struct Reservation {
        admits: Option<bool>,
        released: Cell<Option<(ActionId, ActionId)>>,
    }

    impl FounderArchiveAdmission for Reservation {
        fn reserve(&self, _: ActionId, _: ActionId) -> Option<bool> {
            self.admits
        }

        fn release_unstarted(&self, action: ActionId, target: ActionId) {
            assert!(self.released.replace(Some((action, target))).is_none());
        }
    }

    #[test]
    fn direct_archive_dependency_walk_finishes_a_cold_five_record_chain() {
        let root = ActionId::from_bytes([5; 32]);
        let mut available = BTreeSet::new();
        let mut visits = 0;
        let result = walk_local_member_dependencies(&[root], |id| {
            visits += 1;
            let ordinal = id.as_bytes()[0];
            let predecessor = ActionId::from_bytes([ordinal - 1; 32]);
            if ordinal == 1 || available.contains(&predecessor) {
                available.insert(id);
                Ok(Some(V2OrdinaryReceiveStatus::Retained(ActionState::Valid)))
            } else {
                Ok(Some(V2OrdinaryReceiveStatus::WaitingForDependencies(vec![
                    predecessor,
                ])))
            }
        })
        .unwrap();
        assert_eq!(
            result,
            Some(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
        );
        assert!(available.contains(&root));
        assert_eq!(visits, 9);
    }

    #[test]
    fn direct_archive_dependency_walk_checks_all_local_roots_without_network_fanout() {
        let roots: Vec<_> = (1..=5)
            .map(|byte| ActionId::from_bytes([byte; 32]))
            .collect();
        let shared = ActionId::from_bytes([6; 32]);
        assert!(
            first_founder_missing(&V2OrdinaryReceiveStatus::WaitingForDependencies(
                roots.clone()
            ),)
            .is_none()
        );
        let mut saved = false;
        let mut shared_reads = 0;
        let mut resolved = BTreeSet::new();
        let result = walk_local_member_dependencies(&roots, |id| {
            if id == shared {
                shared_reads += 1;
                saved = true;
            } else if !saved {
                return Ok(Some(V2OrdinaryReceiveStatus::WaitingForDependencies(vec![
                    shared,
                ])));
            }
            resolved.insert(id);
            Ok(Some(V2OrdinaryReceiveStatus::Retained(ActionState::Valid)))
        })
        .unwrap();
        assert_eq!(
            result,
            Some(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
        );
        assert!(roots.into_iter().all(|id| resolved.contains(&id)));
        assert_eq!(shared_reads, 1);
    }

    #[test]
    fn direct_archive_dependency_walk_finishes_dense_durable_ancestry() {
        let action = |byte| ActionId::from_bytes([byte; 32]);
        let mut verified = BTreeSet::new();
        let mut reads = 0;
        let status = walk_local_member_dependencies(&[action(24)], |id| {
            reads += 1;
            let missing: Vec<_> = (1..id.as_bytes()[0])
                .rev()
                .map(action)
                .filter(|parent| !verified.contains(parent))
                .collect();
            if missing.is_empty() {
                verified.insert(id);
                Ok(Some(V2OrdinaryReceiveStatus::Retained(ActionState::Valid)))
            } else {
                Ok(Some(V2OrdinaryReceiveStatus::WaitingForDependencies(
                    missing,
                )))
            }
        })
        .unwrap();
        assert_eq!(
            status,
            Some(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
        );
        assert_eq!(verified.len(), 24);
        assert_eq!(reads, 47);
    }

    #[test]
    fn founder_archive_reservation_tracks_the_visible_effect() {
        let action = ActionId::from_bytes([1; 32]);
        let target = ActionId::from_bytes([2; 32]);

        for (admits, projected, release) in [
            (Some(true), false, true),
            (Some(true), true, false),
            (Some(false), false, false),
        ] {
            let reservation = Reservation {
                admits,
                released: Cell::new(None),
            };
            let result =
                with_founder_archive_reservation(action, Some(target), &reservation, |observe| {
                    let state = if projected {
                        ActionState::Valid
                    } else {
                        ActionState::Invalid(ChatError::Dependency)
                    };
                    observe(&V2OrdinaryReceiveStatus::Retained(state));
                    Err(Refusal::Cancelled)
                });
            assert_eq!(result, Err(Refusal::Cancelled));
            assert_eq!(
                reservation.released.get(),
                release.then_some((action, target))
            );
        }

        let full = Reservation {
            admits: None,
            released: Cell::new(None),
        };
        assert_eq!(
            with_founder_archive_reservation(action, Some(target), &full, |_| {
                panic!("a capacity refusal must not change the ledger")
            }),
            Err(Refusal::Busy)
        );
        assert_eq!(full.released.get(), None);
    }
}

pub(in crate::store) fn first_founder_missing(
    status: &V2OrdinaryReceiveStatus,
) -> Option<ActionId> {
    match status {
        V2OrdinaryReceiveStatus::WaitingForDependencies(missing)
            if missing.len() <= FOUNDER_RECOVERY_BREADTH =>
        {
            missing.first().copied()
        }
        _ => None,
    }
}

/// Restore historical semantics without issuing live projection tokens. The
/// result describes the first requested original, when locally available.
pub(in crate::store) fn replay_local_member_dependencies(
    roots: &[ActionId],
    (verifier, retired, ledger, view): (
        &V2GeneralPrefixVerifier,
        &[&V2FrozenAdmission],
        &mut ChatLedger,
        &ProjectionView,
    ),
    work: usize,
    (store, scope): (&ServerStore, SharedHistoryScope),
    mut saved_dependency: impl FnMut(ActionId) -> Result<Option<MemberSavedOriginal>, Refusal>,
    mut require_current: impl FnMut() -> Result<(), Refusal>,
) -> Result<Option<V2OrdinaryReceiveStatus>, Refusal> {
    if roots.len() > MAX_PARENTS + 1 {
        return Err(Refusal::Failed);
    }
    // Direct ingress runs on the bounded physical read worker. It has no
    // continuation for the old parent's route, so fund one full local walk.
    // Each read still rechecks the original receiver and verifies its bytes.
    let _payment = store
        .archive_bootstrap_policy
        .as_ref()
        .ok_or(Refusal::Unavailable)?
        .pending
        .pool
        .try_reserve(HISTORY_REPLAY_FEE)
        .map_err(|_| Refusal::Busy)?;
    let result = walk_local_member_dependencies(roots, |id| {
        require_current()?;
        replay_local_member_original(
            id,
            (verifier, retired, ledger, view),
            work,
            (store, scope),
            &mut saved_dependency,
            &mut require_current,
        )
    })?;
    require_current()?;
    Ok(result)
}

/// A depth-first stack holds at most one visit or retry per ActionId. Move a
/// queued visit ahead of the child that needs it without duplicating dense DAGs.
pub(super) struct LocalDependencyStack(pub(super) Vec<(ActionId, bool)>, usize);

impl LocalDependencyStack {
    pub(super) fn new(roots: &[ActionId]) -> Self {
        let mut pending = Vec::with_capacity(HISTORY_PENDING_IDS);
        for &root in roots.iter().rev() {
            if !pending.contains(&(root, false)) {
                pending.push((root, false));
            }
        }
        Self(pending, HISTORY_PENDING_IDS)
    }

    pub(super) fn exact_sorted(roots: &[ActionId], limit: usize) -> Result<Self, Refusal> {
        if roots.len() > limit || roots.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(Refusal::Failed);
        }
        let mut pending = Vec::with_capacity(limit);
        pending.extend(roots.iter().rev().map(|&id| (id, false)));
        Ok(Self(pending, limit))
    }

    pub(super) fn descend(
        &mut self,
        child: ActionId,
        missing: &[ActionId],
        satisfied: impl Fn(ActionId) -> bool,
    ) -> Result<(), Refusal> {
        if missing.iter().any(|parent| {
            *parent == child || self.0.iter().any(|(id, retry)| *retry && id == parent)
        }) {
            return Err(Refusal::Unavailable);
        }
        let new_visits = missing
            .iter()
            .enumerate()
            .filter(|&(index, parent)| {
                !satisfied(*parent)
                    && !missing[..index].contains(parent)
                    && !self.0.iter().any(|(id, _)| id == parent)
            })
            .count();
        if self.0.len() + 1 + new_visits > self.1 {
            return Err(Refusal::Unavailable);
        }
        self.0.push((child, true));
        for &parent in missing.iter().rev().filter(|&&id| !satisfied(id)) {
            if let Some(queued) = self.0.iter().position(|(id, _)| *id == parent) {
                self.0.remove(queued);
            }
            self.0.push((parent, false));
        }
        Ok(())
    }
}

fn walk_local_member_dependencies(
    roots: &[ActionId],
    mut read: impl FnMut(ActionId) -> Result<Option<V2OrdinaryReceiveStatus>, Refusal>,
) -> Result<Option<V2OrdinaryReceiveStatus>, Refusal> {
    let mut pending = LocalDependencyStack::new(roots);
    let mut first = None;
    let mut completed = BTreeSet::new();
    let mut reads = 0;
    while let Some((id, retry)) = pending.0.pop() {
        if completed.contains(&id) {
            continue;
        }
        if reads == HISTORY_DEPENDENCY_READS {
            break;
        }
        reads += 1;
        let Some(status) = read(id)? else {
            continue;
        };
        if status == V2OrdinaryReceiveStatus::Retained(ActionState::Valid) {
            completed.insert(id);
        }
        if let V2OrdinaryReceiveStatus::WaitingForDependencies(missing) = &status {
            // A queued retry is CPU work, not a prerequisite fetch budget.
            // Descend first so the walk can establish verified leaf progress.
            if !retry {
                pending.descend(id, missing, |parent| completed.contains(&parent))?;
            }
        }
        if roots.first() == Some(&id) {
            first = Some(status);
        }
    }
    Ok(first)
}

/// The signed original is reverified in the current prefix on every
/// traversal step. A durable record alone is not a semantic witness.
fn replay_local_member_original(
    id: ActionId,
    (verifier, retired, ledger, view): (
        &V2GeneralPrefixVerifier,
        &[&V2FrozenAdmission],
        &mut ChatLedger,
        &ProjectionView,
    ),
    work: usize,
    (store, scope): (&ServerStore, SharedHistoryScope),
    saved_dependency: &mut impl FnMut(ActionId) -> Result<Option<MemberSavedOriginal>, Refusal>,
    require_current: &mut impl FnMut() -> Result<(), Refusal>,
) -> Result<Option<V2OrdinaryReceiveStatus>, Refusal> {
    let own = saved_dependency(id)?;
    require_current()?;
    if let Some(saved) = own
        .as_ref()
        .filter(|saved| verifier.admission(saved.receipt().admission).is_ok())
    {
        let action = saved.action();
        if action.id() != id || &action.spec().realm != ledger.realm() {
            return Err(Refusal::Stale);
        }
        return verifier
            .replay_current_outbox_ancestor(
                ledger,
                view,
                (action, saved.receipt().admission),
                retired,
                work,
            )
            .map(Some)
            .map_err(receive_ledger_refusal);
    }
    if !store
        .has_mounted_shared_history()
        .map_err(|_| Refusal::Busy)?
    {
        return Ok(None);
    }
    // Only a selected record can supply a removed author's original.
    // This lookup cannot recover a pending writer or select a WAL.
    let archived = store
        .read_selected_shared_history_ancestor(scope, id)
        .map_err(|_| Refusal::Failed)?;
    require_current()?;
    let Some(archived) = archived else {
        return Ok(None);
    };
    let action = archived.original.action();
    if action.id() != id || &action.spec().realm != ledger.realm() {
        return Err(Refusal::Stale);
    }
    let selected = V2LocalArchiveOriginal::from_trusted_local_selection(
        action,
        archived.admission,
        archived.original.original_bytes(),
    )
    .map_err(|_| Refusal::Failed)?;
    verifier
        .replay_locally_selected_ancestor(ledger, view, selected, retired, work)
        .map(Some)
        .map_err(receive_ledger_refusal)
}

/// Exact unfinished local ancestor traversal for a single paid staging ledger.
/// Each stored retry is behind its parents in stack order.
pub(in crate::store) struct LocalHistoryDependencyReplay {
    pending: LocalDependencyStack,
    reads: usize,
    exact_miss: Option<ActionId>,
}

impl LocalHistoryDependencyReplay {
    pub(in crate::store) fn new(roots: Vec<ActionId>) -> Result<Self, Refusal> {
        if roots.len() > MAX_PARENTS + 1 {
            return Err(Refusal::Failed);
        }
        Ok(Self {
            pending: LocalDependencyStack::new(&roots),
            reads: 0,
            exact_miss: None,
        })
    }

    pub(in crate::store) fn exact_miss(&self) -> Option<ActionId> {
        self.exact_miss
    }

    pub(in crate::store) fn clear_exact_miss(&mut self) {
        self.exact_miss = None;
    }

    /// Return false only after yielding a bounded slice. Missing data or
    /// reaching the total bound defers the action to a future sweep.
    pub(in crate::store) fn advance(
        &mut self,
        (verifier, retired, ledger, view): (
            &V2GeneralPrefixVerifier,
            &[&V2FrozenAdmission],
            &mut ChatLedger,
            &ProjectionView,
        ),
        work: usize,
        archive: (&ServerStore, SharedHistoryScope),
        mut saved_dependency: impl FnMut(ActionId) -> Result<Option<MemberSavedOriginal>, Refusal>,
        mut require_current: impl FnMut() -> Result<(), Refusal>,
    ) -> Result<bool, Refusal> {
        let mut reads = 0;
        while let Some(&(id, retry)) = self.pending.0.last() {
            require_current()?;
            if verifier.locally_usable_ordinary_dependency(ledger, id) {
                self.pending.0.pop();
                continue;
            }
            if self.reads == HISTORY_DEPENDENCY_READS {
                return Err(Refusal::Unavailable);
            }
            if reads == LOCAL_DEPENDENCY_READS {
                return Ok(false);
            }
            reads += 1;
            self.reads += 1;
            let Some(status) = replay_local_member_original(
                id,
                (verifier, retired, ledger, view),
                work,
                archive,
                &mut saved_dependency,
                &mut require_current,
            )?
            else {
                require_current()?;
                self.exact_miss = Some(id);
                return Err(Refusal::Unavailable);
            };
            self.pending.0.pop();
            match status {
                V2OrdinaryReceiveStatus::Retained(ActionState::Valid) => {}
                V2OrdinaryReceiveStatus::WaitingForDependencies(missing) => {
                    if retry {
                        return Err(Refusal::Unavailable);
                    }
                    self.pending.descend(id, &missing, |parent| {
                        verifier.locally_usable_ordinary_dependency(ledger, parent)
                    })?;
                }
                V2OrdinaryReceiveStatus::Retained(_) => return Err(Refusal::Failed),
            }
        }
        require_current()?;
        Ok(true)
    }
}

pub(in crate::store) const fn archive_ancestry_fee(limit: usize) -> usize {
    limit * 256 + (MAX_PARENTS + 1) * 32
}

/// Verify the complete bounded closure against the exact originals already
/// selected in the local archive or own outbox. The caller also verifies
/// the original row and pays for both sets before entering the walk.
pub(in crate::store) fn require_durable_archive_dependencies(
    (ledger, verifier): (&ChatLedger, &V2GeneralPrefixVerifier),
    original: &SignedAction,
    (store, scope): (&ServerStore, SharedHistoryScope),
    limit: usize,
    mut read_own: impl FnMut(ActionId) -> Result<Option<MemberSavedOriginal>, Refusal>,
    mut require_current: impl FnMut() -> Result<(), Refusal>,
) -> Result<(), Refusal> {
    let action = original.id();
    let target = original.spec().body.create().unwrap_or(action);
    let mut ancestors = original.dependencies();
    if ancestors.is_empty() {
        return require_current();
    }
    let mut selected = BTreeSet::from([action]);
    let mut pending = BTreeSet::new();
    loop {
        for dependency in ancestors {
            if !selected.contains(&dependency) && !pending.contains(&dependency) {
                if selected.len() + pending.len() >= limit {
                    return Err(Refusal::Unavailable);
                }
                pending.insert(dependency);
            }
        }
        let Some(dependency) = pending.pop_first() else {
            break;
        };
        require_current()?;
        let archived = store
            .read_selected_shared_history_ancestor(scope, dependency)
            .map_err(|_| Refusal::Failed)?;
        let own = if archived.is_none() && dependency != target {
            read_own(dependency)?
                .filter(|record| verifier.admission(record.receipt().admission).is_ok())
        } else {
            None
        };
        let (signed, admission) = archived
            .as_ref()
            .map(|record| (record.original.action(), record.admission))
            .or_else(|| {
                own.as_ref()
                    .map(|record| (record.action(), record.receipt().admission))
            })
            .ok_or(Refusal::Unavailable)?;
        if signed.id() != dependency || !ledger.matches_valid_ordinary_original(signed, admission) {
            return Err(Refusal::Stale);
        }
        selected.insert(dependency);
        ancestors = signed.dependencies();
    }
    require_current()
}

pub(in crate::store) fn archive_founder_member<R: CryptoRngCore>(
    (store, scope): (&ServerStore, SharedHistoryScope),
    original: &SelectedV2MemberEnvelope,
    ledger: (&ChatLedger, &V2GeneralPrefixVerifier),
    rng: &mut R,
    mut read_own: impl FnMut(ActionId, &mut R) -> Result<Option<MemberSavedOriginal>, Refusal>,
    mut require_current: impl FnMut() -> Result<(), AppError>,
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
        .try_reserve(SHARED_HISTORY_RECORD_FEE + archive_ancestry_fee(FOUNDER_RECEIVE_ACTIONS))?;
    require_durable_archive_dependencies(
        ledger,
        original.action_data(),
        (store, scope),
        FOUNDER_RECEIVE_ACTIONS,
        |dependency| read_own(dependency, rng),
        || require_current().map_err(|_| Refusal::Stale),
    )
    .map_err(|_| refused())?;
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

#[expect(
    clippy::large_enum_variant,
    reason = "The paid publication handoff moves the original submission inline."
)]
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

/// Local native bookkeeping for one actual authenticated received mutation.
/// It supplies capacity only. Signature, selected member and scope checks
/// remain the responsibility of the original receive owner.
pub trait FounderArchiveAdmission {
    /// Some(true) reserves a new exact slot. Some(false) finds the same one.
    fn reserve(&self, action: ActionId, target: ActionId) -> Option<bool>;

    /// Release a newly reserved slot before any effect or archive write.
    fn release_unstarted(&self, action: ActionId, target: ActionId);
}

/// One reusable cache, bound to its original subscription and channel.
/// Open a replacement only after the stopped or full cache is dropped.
pub(in crate::store) struct FounderReceiveLedger {
    pub(in crate::store) view: ProjectionView,
    pub(in crate::store) ledger: ChatLedger,
    pub(in crate::store) session: FounderReceiveSession,
    _fee: CatalogReadReservation,
}

impl FounderReceiveLedger {
    pub(in crate::store) fn capture<'a>(
        slot: &'a mut Option<Self>,
        session: &FounderReceiveSession,
        (group, channel): (&[u8], u128),
        reserve: impl FnOnce() -> Result<CatalogReadReservation, Refusal>,
        begin: impl FnOnce(&mut ChatLedger) -> Result<ProjectionView, Refusal>,
    ) -> Result<&'a mut Self, Refusal> {
        if session.signal.is_cancelled() {
            return Err(Refusal::Stale);
        }
        if slot
            .as_ref()
            .is_some_and(|cache| cache.session.signal.is_cancelled())
        {
            *slot = None;
        }
        if slot.is_none() {
            let fee = reserve()?;
            let realm = Realm::new(group, channel).map_err(|_| Refusal::Failed)?;
            let mut ledger = ChatLedger::new(
                realm,
                LedgerLimits {
                    max_actions: FOUNDER_RECEIVE_ACTIONS,
                    max_action_bytes: FOUNDER_RECEIVE_BYTES,
                },
            )
            .map_err(|_| Refusal::Failed)?;
            let mut view = begin(&mut ledger)?;
            if !view.try_ready() {
                return Err(Refusal::Busy);
            }
            *slot = Some(Self {
                view,
                ledger,
                session: session.clone(),
                _fee: fee,
            });
        }
        slot.as_mut()
            .filter(|cache| {
                Arc::ptr_eq(&cache.session.identity, &session.identity)
                    && cache.ledger.realm().channel() == channel
            })
            .ok_or(Refusal::Stale)
    }
}

pub(in crate::store) enum FounderReceiveOperation<'a> {
    Subscription,
    Frame(
        &'a NativeChatMemberFrame,
        &'a FounderReceiveSession,
        Option<NativeChatMemberDependencyExchange>,
        &'a dyn FounderArchiveAdmission,
    ),
    Recover(
        &'a NativeTextFounderDependencyResponse,
        &'a FounderReceiveSession,
        Option<NativeChatMemberDependencyExchange>,
        &'a dyn FounderArchiveAdmission,
    ),
    Dependency(&'a NativeChatMemberDependencyIncoming),
    Read(ActionId),
    ArchivePage(SharedHistoryActionQuery),
    ArchiveMessage(ActionId),
    TargetMessage(ActionId),
    SelectedTarget(ActionId),
    Audit(SharedHistoryActionQuery),
}

/// A founder's paid page of checked local archive records. The scope is a
/// negative observation only. The runtime and archive remain with the owner.
pub struct NativeTextFounderAudit {
    page: Option<SelectedSharedHistoryAuditPage>,
    signal: RequestCancellation,
}

impl NativeTextFounderAudit {
    pub(in crate::store) fn new(
        page: Option<SelectedSharedHistoryAuditPage>,
        signal: RequestCancellation,
    ) -> Self {
        Self { page, signal }
    }

    pub fn is_current_cached(&self) -> bool {
        !self.signal.is_cancelled()
    }

    /// Permit inspection only while the original founder source and
    /// covered membership remain live. None means the archive was unavailable.
    pub fn with_records<T>(
        &self,
        read: impl FnOnce(Option<&[(OwnerFactId, AssembledActionOriginal)]>, Option<ActionId>) -> T,
    ) -> Result<T, Refusal> {
        if !self.is_current_cached() {
            return Err(Refusal::Stale);
        }
        let result = read(
            self.page.as_ref().map(|page| page.records.as_slice()),
            self.page.as_ref().and_then(|page| page.next),
        );
        if !self.is_current_cached() {
            return Err(Refusal::Stale);
        }
        Ok(result)
    }
}

/// The original founder receiver's update for a verified action.
pub struct NativeTextFounderReceived {
    pub action: ActionId,
    pub projection_target: ActionId,
    /// Whether the selected archive covered the action. No value means this
    /// vault has no mounted shared history namespace.
    pub archived: Option<bool>,
}

impl NativeTextFounderReceived {
    pub(in crate::store) fn from_original(
        original: &SelectedV2MemberEnvelope,
        archived: Option<bool>,
    ) -> Self {
        let action = original.action_data();
        Self {
            action: action.id(),
            projection_target: action.spec().body.create().unwrap_or_else(|| action.id()),
            archived,
        }
    }
}

#[expect(
    clippy::large_enum_variant,
    reason = "The receive result moves one paid dependency exchange inline."
)]
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
    Recovered(NativeTextFounderFrameOutcome),
    Read(
        Option<(
            GuardedMessageProjection,
            RequestCancellation,
            CatalogReadReservation,
        )>,
    ),
    ArchivePage(Option<SharedHistoryActionPage>),
    ArchiveMessage(Option<FounderArchivedMessage>),
    SelectedTarget(Option<ActionId>),
    Audit(NativeTextFounderAudit),
    RecoveredProjection(Option<NativeTextJoinedMemberProjection>),
}
