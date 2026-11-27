//! One paid founder receiver ledger in the original selected text host.
use super::*;
use crate::store::persistence::v2_member_transport::{
    first_founder_missing, prepare_founder_dependency_exchange, prepare_founder_dependency_reply,
    project_founder_member, FounderReceiveOperation, FounderReceiveOutcome, FounderReceiveSession,
    NativeTextFounderAudit, NativeTextFounderFrameOutcome, NativeTextFounderReceived,
    FOUNDER_DEPENDENCY_REPLY_FEE, FOUNDER_RECEIVE_ACTIONS as ACTION_LIMIT,
    FOUNDER_RECEIVE_BYTES as ACTION_BYTES, FOUNDER_RECEIVE_LEDGER_FEE as LEDGER_FEE,
    FOUNDER_RECEIVE_WORK as WORK_LIMIT,
};
use crate::store::persistence::v2_shared_history::SharedHistoryActionQuery;
use catcoms_replication::chat::{ActionState, V2OrdinaryReceiveStatus};

mod archive;
use archive::FounderHistory;

pub(super) struct FounderReceivedLedger {
    view: ProjectionView,
    ledger: ChatLedger,
    session: FounderReceiveSession,
    _fee: CatalogReadReservation,
}

impl V2MemberLane {
    fn read_live_projection(
        &self,
        channel: u128,
        create: ActionId,
        pool: &crate::store::critical_namespaces::read_pool::CriticalCatalogReadPool,
    ) -> Result<FounderReceiveOutcome, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let Some(received) = self.received.as_ref().filter(|received| {
            !received.session.signal.is_cancelled()
                && received.ledger.realm().channel() == channel
                && received.ledger.action(create).is_some()
        }) else {
            return Ok(FounderReceiveOutcome::Read(None));
        };
        let fee = pool
            .try_reserve(2 * MAX_ACTION_BYTES + 32 * 1024)
            .map_err(|_| Refusal::Busy)?;
        let projection = received
            .ledger
            .guarded_projection(create, WORK_LIMIT)
            .map_err(|_| Refusal::Failed)?;
        Ok(FounderReceiveOutcome::Read(Some((
            projection,
            received.session.signal.clone(),
            fee,
        ))))
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    pub(in crate::store) fn run_founder_member_receive(
        &mut self,
        layout: &V2CurrentLayoutStore,
        channel: u128,
        operation: FounderReceiveOperation<'_>,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
        access: FounderAccess<'_>,
    ) -> Result<FounderReceiveOutcome, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        access
            .require(&self.origin)
            .map_err(|_| Refusal::Unavailable)?;
        if caller.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        let Some(governed) = self
            .governed_add
            .as_deref_mut()
            .filter(|add| add.installed.is_some())
        else {
            return match operation {
                FounderReceiveOperation::Read(_) => Ok(FounderReceiveOutcome::Read(None)),
                FounderReceiveOperation::ArchivePage(_) => {
                    Ok(FounderReceiveOutcome::ArchivePage(None))
                }
                FounderReceiveOperation::ArchiveMessage(_)
                | FounderReceiveOperation::TargetMessage(_) => {
                    Ok(FounderReceiveOutcome::ArchiveMessage(None))
                }
                _ => Err(Refusal::Unavailable),
            };
        };
        let store = layout.checked_store().map_err(|_| Refusal::Unavailable)?;
        let lane = self.member.as_deref_mut().ok_or(Refusal::Unavailable)?;
        lane.validate_accounts(store)
            .map_err(|_| Refusal::Unavailable)?;
        let verifier = lane.verifier.as_ref().ok_or(Refusal::Unavailable)?;
        check_governed_member(&self.owner, governed, &lane.domain, verifier, store)
            .map_err(|_| Refusal::Stale)?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or(Refusal::Unavailable)?
            .pending
            .pool;
        match operation {
            FounderReceiveOperation::Subscription => {
                let fee = pool.try_reserve(8 * 1024).map_err(|_| Refusal::Busy)?;
                let (topic, observation) = self
                    .owner
                    .prepare_governed_member_subscription(store, verifier, channel)
                    .map_err(|_| Refusal::Unavailable)?;
                Ok(FounderReceiveOutcome::Subscription(topic, observation, fee))
            }
            FounderReceiveOperation::Dependency(incoming) => {
                let fee = pool
                    .try_reserve(FOUNDER_DEPENDENCY_REPLY_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let provider = self
                    .owner
                    .select_governed_dependency_access(store, verifier, channel)
                    .map_err(|_| Refusal::Unavailable)?;
                let reply = prepare_founder_dependency_reply(
                    provider,
                    incoming,
                    fee,
                    caller,
                    rng,
                    |id, rng| {
                        let history = FounderHistory::new(
                            store,
                            verifier,
                            self.owner
                                .borrow_current()
                                .map_err(|_| Refusal::Unavailable)?,
                            channel,
                            lane.admission,
                            lane.outbox_context(),
                        )?;
                        let mount = store
                            .member_outbox_mount()
                            .map_err(|_| Refusal::Unavailable)?;
                        let key = member_outbox_key(store).map_err(|_| Refusal::Unavailable)?;
                        history.saved(&mount, &key, id, rng)
                    },
                )?;
                check_governed_member(&self.owner, governed, &lane.domain, verifier, store)
                    .map_err(|_| Refusal::Stale)?;
                Ok(FounderReceiveOutcome::Dependency(reply))
            }
            FounderReceiveOperation::Frame(frame, session, carrier, admit_mutation) => {
                if session.signal.is_cancelled() {
                    return Err(Refusal::Cancelled);
                }
                let _fee = pool
                    .try_reserve(FOUNDER_TRANSPORT_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let (topic, _, data) = frame.current_data().ok_or(Refusal::Stale)?;
                let original = self
                    .owner
                    .open_governed_member_frame(store, verifier, channel, topic, data)
                    .map_err(|_| Refusal::Failed)?;
                if lane
                    .received
                    .as_ref()
                    .is_some_and(|ledger| ledger.session.signal.is_cancelled())
                {
                    lane.received = None;
                }
                if lane.received.is_none() {
                    let fee = pool.try_reserve(LEDGER_FEE).map_err(|_| Refusal::Busy)?;
                    let realm = Realm::new(&lane.group, channel).map_err(|_| Refusal::Failed)?;
                    let mut ledger = ChatLedger::new(
                        realm,
                        LedgerLimits {
                            max_actions: ACTION_LIMIT,
                            max_action_bytes: ACTION_BYTES,
                        },
                    )
                    .map_err(|_| Refusal::Failed)?;
                    let mut view = lane
                        .domain
                        .begin_channel_view(&mut ledger)
                        .map_err(|_| Refusal::Unavailable)?;
                    if !view.try_ready() {
                        return Err(Refusal::Busy);
                    }
                    lane.received = Some(FounderReceivedLedger {
                        ledger,
                        view,
                        session: session.clone(),
                        _fee: fee,
                    });
                }
                let outbox = lane.outbox_context();
                let received = lane
                    .received
                    .as_mut()
                    .filter(|ledger| {
                        Arc::ptr_eq(&ledger.session.identity, &session.identity)
                            && ledger.ledger.realm().channel() == channel
                    })
                    .ok_or(Refusal::Stale)?;
                if session.signal.is_cancelled() || frame.current_data().is_none() {
                    return Err(Refusal::Stale);
                }
                let history = FounderHistory::new(
                    store,
                    verifier,
                    self.owner
                        .borrow_current()
                        .map_err(|_| Refusal::Unavailable)?,
                    channel,
                    lane.admission,
                    outbox,
                )?;
                let mut require_current = || {
                    check_governed_member(&self.owner, governed, &lane.domain, verifier, store)
                        .map_err(|_| Refusal::Stale)?;
                    if session.signal.is_cancelled() || frame.current_data().is_none() {
                        return Err(Refusal::Stale);
                    }
                    Ok(())
                };
                let status = project_founder_member(
                    &original,
                    (verifier, &lane.retired_history),
                    &mut received.ledger,
                    &received.view,
                    (store, history.scope, admit_mutation),
                    |id| {
                        let mount = store
                            .member_outbox_mount()
                            .map_err(|_| Refusal::Unavailable)?;
                        let key = member_outbox_key(store).map_err(|_| Refusal::Unavailable)?;
                        history.saved(&mount, &key, id, rng)
                    },
                    &mut require_current,
                )?;
                if let (Some(carrier), Some(id)) = (carrier, first_founder_missing(&status)) {
                    require_current()?;
                    let fee = pool
                        .try_reserve(FOUNDER_DEPENDENCY_REPLY_FEE)
                        .map_err(|_| Refusal::Busy)?;
                    let access = self
                        .owner
                        .select_governed_dependency_access(store, verifier, channel)
                        .map_err(|_| Refusal::Unavailable)?;
                    let request = prepare_founder_dependency_exchange(
                        access,
                        carrier,
                        (session, None),
                        &original,
                        id,
                        fee,
                        rng,
                    )?;
                    require_current()?;
                    return Ok(FounderReceiveOutcome::Frame(
                        NativeTextFounderFrameOutcome::NeedsDependency(request),
                    ));
                }
                if status != V2OrdinaryReceiveStatus::Retained(ActionState::Valid) {
                    return Ok(FounderReceiveOutcome::Frame(
                        NativeTextFounderFrameOutcome::Unresolved,
                    ));
                }
                let archived = history
                    .save(&original, &received.ledger, rng, || {
                        require_current().map_err(|_| refused())
                    })
                    .unwrap_or(Some(false));
                require_current()?;
                Ok(FounderReceiveOutcome::Frame(
                    NativeTextFounderFrameOutcome::Retained(
                        NativeTextFounderReceived::from_original(&original, archived),
                    ),
                ))
            }
            FounderReceiveOperation::Recover(response, session, carrier, admit_mutation) => {
                response.require_current(session)?;
                let _fee = pool
                    .try_reserve(FOUNDER_TRANSPORT_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let access = self
                    .owner
                    .select_governed_dependency_access(store, verifier, channel)
                    .map_err(|_| Refusal::Unavailable)?;
                let Some(original) = response.open(access, session)? else {
                    return Ok(FounderReceiveOutcome::Recovered(
                        NativeTextFounderFrameOutcome::Unresolved,
                    ));
                };
                let outbox = lane.outbox_context();
                let received = lane
                    .received
                    .as_mut()
                    .filter(|ledger| {
                        Arc::ptr_eq(&ledger.session.identity, &session.identity)
                            && ledger.ledger.realm().channel() == channel
                    })
                    .ok_or(Refusal::Stale)?;
                let history = FounderHistory::new(
                    store,
                    verifier,
                    self.owner
                        .borrow_current()
                        .map_err(|_| Refusal::Unavailable)?,
                    channel,
                    lane.admission,
                    outbox,
                )?;
                let mut require_current = || {
                    check_governed_member(&self.owner, governed, &lane.domain, verifier, store)
                        .map_err(|_| Refusal::Stale)?;
                    response.require_current(session)
                };
                let status = project_founder_member(
                    &original,
                    (verifier, &lane.retired_history),
                    &mut received.ledger,
                    &received.view,
                    (store, history.scope, admit_mutation),
                    |id| {
                        let mount = store
                            .member_outbox_mount()
                            .map_err(|_| Refusal::Unavailable)?;
                        let key = member_outbox_key(store).map_err(|_| Refusal::Unavailable)?;
                        history.saved(&mount, &key, id, rng)
                    },
                    &mut require_current,
                )?;
                if let (Some(carrier), Some(id)) = (carrier, first_founder_missing(&status)) {
                    require_current()?;
                    let fee = pool
                        .try_reserve(FOUNDER_DEPENDENCY_REPLY_FEE)
                        .map_err(|_| Refusal::Busy)?;
                    let access = self
                        .owner
                        .select_governed_dependency_access(store, verifier, channel)
                        .map_err(|_| Refusal::Unavailable)?;
                    let request = prepare_founder_dependency_exchange(
                        access,
                        carrier,
                        (session, Some(response)),
                        &original,
                        id,
                        fee,
                        rng,
                    )?;
                    require_current()?;
                    return Ok(FounderReceiveOutcome::Recovered(
                        NativeTextFounderFrameOutcome::NeedsDependency(request),
                    ));
                }
                if status != V2OrdinaryReceiveStatus::Retained(ActionState::Valid) {
                    return Ok(FounderReceiveOutcome::Recovered(
                        NativeTextFounderFrameOutcome::Unresolved,
                    ));
                }
                let archived = history
                    .save(&original, &received.ledger, rng, || {
                        require_current().map_err(|_| refused())
                    })
                    .unwrap_or(Some(false));
                require_current()?;
                Ok(FounderReceiveOutcome::Recovered(
                    NativeTextFounderFrameOutcome::Retained(
                        NativeTextFounderReceived::from_original(&original, archived),
                    ),
                ))
            }
            FounderReceiveOperation::Read(create) => {
                lane.read_live_projection(channel, create, pool)
            }
            FounderReceiveOperation::ArchivePage(query) => {
                let history = FounderHistory::new(
                    store,
                    verifier,
                    self.owner
                        .borrow_current()
                        .map_err(|_| Refusal::Unavailable)?,
                    channel,
                    lane.admission,
                    lane.outbox_context(),
                )?;
                history
                    .page(SharedHistoryActionQuery {
                        target: None,
                        ..query
                    })
                    .map(FounderReceiveOutcome::ArchivePage)
            }
            FounderReceiveOperation::ArchiveMessage(create)
            | FounderReceiveOperation::TargetMessage(create) => {
                let archived = store
                    .has_mounted_shared_history()
                    .map_err(|_| Refusal::Busy)?;
                if !archived && matches!(operation, FounderReceiveOperation::TargetMessage(_)) {
                    return lane.read_live_projection(channel, create, pool);
                }
                let (_, observation) = self
                    .owner
                    .prepare_governed_member_subscription(store, verifier, channel)
                    .map_err(|_| Refusal::Stale)?;
                let scope = observation.constrain(caller.clone());
                let history = FounderHistory::new(
                    store,
                    verifier,
                    self.owner
                        .borrow_current()
                        .map_err(|_| Refusal::Unavailable)?,
                    channel,
                    lane.admission,
                    lane.outbox_context(),
                )?;
                let result = history.message(
                    &lane.domain,
                    &lane.retired_history,
                    create,
                    scope,
                    rng,
                    pool,
                )?;
                check_governed_member(&self.owner, governed, &lane.domain, verifier, store)
                    .map_err(|_| Refusal::Stale)?;
                Ok(FounderReceiveOutcome::ArchiveMessage(result))
            }
            FounderReceiveOperation::SelectedTarget(action) => {
                let (_, observation) = self
                    .owner
                    .prepare_governed_member_subscription(store, verifier, channel)
                    .map_err(|_| Refusal::Stale)?;
                let signal = observation.constrain(caller.clone());
                let history = FounderHistory::new(
                    store,
                    verifier,
                    self.owner
                        .borrow_current()
                        .map_err(|_| Refusal::Unavailable)?,
                    channel,
                    lane.admission,
                    lane.outbox_context(),
                )?;
                if signal.is_cancelled() {
                    return Err(Refusal::Cancelled);
                }
                let target = store
                    .selected_shared_history_target(history.scope, action)
                    .map_err(|_| Refusal::Failed)?;
                check_governed_member(&self.owner, governed, &lane.domain, verifier, store)
                    .map_err(|_| Refusal::Stale)?;
                if signal.is_cancelled() {
                    return Err(Refusal::Cancelled);
                }
                Ok(FounderReceiveOutcome::SelectedTarget(target))
            }
            FounderReceiveOperation::Audit(query) => {
                let (_, observation) = self
                    .owner
                    .prepare_governed_member_subscription(store, verifier, channel)
                    .map_err(|_| Refusal::Stale)?;
                let signal = observation.constrain(caller.clone());
                let history = FounderHistory::new(
                    store,
                    verifier,
                    self.owner
                        .borrow_current()
                        .map_err(|_| Refusal::Unavailable)?,
                    channel,
                    lane.admission,
                    lane.outbox_context(),
                )?;
                let page = store
                    .selected_shared_history_audit_page(history.scope, query, &signal)
                    .map_err(|_| Refusal::Failed)?;
                check_governed_member(&self.owner, governed, &lane.domain, verifier, store)
                    .map_err(|_| Refusal::Stale)?;
                Ok(FounderReceiveOutcome::Audit(NativeTextFounderAudit::new(
                    page, signal,
                )))
            }
        }
    }
}
