//! One paid founder receiver ledger in the original selected text host.
use super::*;
use crate::store::persistence::v2_member_transport::{
    FounderReceiveOperation, FounderReceiveOutcome, FounderReceiveSession,
    NativeTextFounderReceived,
};
use crate::store::persistence::v2_shared_history::SharedHistoryActionQuery;
use catcoms_replication::chat::{ActionState, V2OrdinaryReceiveStatus};

mod archive;
use archive::FounderHistory;

const ACTION_LIMIT: usize = 64;
const ACTION_BYTES: usize = 1024 * 1024;
const WORK_LIMIT: usize = 4 * ACTION_LIMIT + MAX_PARENTS;
const MAX_SAVED_DEPENDENCIES: usize = 4;
const LEDGER_FEE: usize = 2 * ACTION_BYTES
    + ACTION_LIMIT * (MAX_PARENTS + 2) * 256
    + V2GeneralPrefixVerifier::ordinary_authorization_temporary_layout_bytes()
    + 16 * 1024;

pub(super) struct FounderReceivedLedger {
    view: ProjectionView,
    ledger: ChatLedger,
    session: FounderReceiveSession,
    _fee: CatalogReadReservation,
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
                FounderReceiveOperation::ArchiveMessage(_) => {
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
            FounderReceiveOperation::Frame(frame, session) => {
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
                let mut status = original
                    .project_from_ledger(verifier, &mut received.ledger, &received.view, WORK_LIMIT)
                    .map_err(|_| Refusal::Failed)?;
                if let V2OrdinaryReceiveStatus::WaitingForDependencies(missing) = &status {
                    if missing.len() > MAX_SAVED_DEPENDENCIES {
                        return Ok(FounderReceiveOutcome::Frame(None));
                    }
                    let mount = store
                        .member_outbox_mount()
                        .map_err(|_| Refusal::Unavailable)?;
                    let key = member_outbox_key(store).map_err(|_| Refusal::Unavailable)?;
                    let admission = verifier
                        .admission(lane.admission)
                        .map_err(|_| Refusal::Stale)?;
                    let mut replayed = false;
                    for &action in missing {
                        if session.signal.is_cancelled() || frame.current_data().is_none() {
                            return Err(Refusal::Stale);
                        }
                        let Some(saved) = history.saved(&mount, &key, action, rng)? else {
                            continue;
                        };
                        check_governed_member(&self.owner, governed, &lane.domain, verifier, store)
                            .map_err(|_| Refusal::Stale)?;
                        if session.signal.is_cancelled() || frame.current_data().is_none() {
                            return Err(Refusal::Stale);
                        }
                        replayed |= verifier
                            .replay_ordinary_history_original(
                                &mut received.ledger,
                                &received.view,
                                saved.action(),
                                &admission,
                                &[],
                                WORK_LIMIT,
                            )
                            .map_err(|_| Refusal::Failed)?
                            == V2OrdinaryReceiveStatus::Retained(ActionState::Valid);
                    }
                    if !replayed {
                        return Ok(FounderReceiveOutcome::Frame(None));
                    }
                    status = original
                        .project_from_ledger(
                            verifier,
                            &mut received.ledger,
                            &received.view,
                            WORK_LIMIT,
                        )
                        .map_err(|_| Refusal::Failed)?;
                }
                if status != V2OrdinaryReceiveStatus::Retained(ActionState::Valid) {
                    return Ok(FounderReceiveOutcome::Frame(None));
                }
                let action = original.action_data();
                check_governed_member(&self.owner, governed, &lane.domain, verifier, store)
                    .map_err(|_| Refusal::Stale)?;
                if session.signal.is_cancelled() || frame.current_data().is_none() {
                    return Err(Refusal::Stale);
                }
                let archived = history.save(&original, rng).unwrap_or(Some(false));
                Ok(FounderReceiveOutcome::Frame(Some(
                    NativeTextFounderReceived {
                        action: action.id(),
                        projection_target: action
                            .spec()
                            .body
                            .create()
                            .unwrap_or_else(|| action.id()),
                        archived,
                    },
                )))
            }
            FounderReceiveOperation::Read(create) => {
                let Some(received) = lane.received.as_ref().filter(|received| {
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
            FounderReceiveOperation::ArchiveMessage(create) => {
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
                let result = history.message(&lane.domain, create, scope, rng, pool)?;
                check_governed_member(&self.owner, governed, &lane.domain, verifier, store)
                    .map_err(|_| Refusal::Stale)?;
                Ok(FounderReceiveOutcome::ArchiveMessage(result))
            }
        }
    }
}
