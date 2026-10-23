//! Inactive authenticated child creation/adoption. Renderer purpose/role fields are lookup
//! hints only. One actual native snapshot (not a polling loop) supplies an incoming action.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, Operation, PhysicalHold};
use super::context::Context;
use super::factory::Refused;
use super::ledger::{ConfirmationPurpose, Purpose, ResolvedRow, RowId, Status};
use super::requests::{confirmation::Request, Kind};
use catcoms_sync::voice_v2::crypto_actions::{
    CryptoAction, CryptoActionSnapshotPermit, CryptoOperation,
};
use catcoms_sync::voice_v2::resources::{Credit, ResourceKind};
use catcoms_sync::voice_v2::VoiceError;
use std::sync::Arc;

// A child may have no attached Phase yet. Pay this actual task from ordinary native
// capacity BEFORE snapshot/factory work. The separately paid4KiB snapshot and existing4KiB
// factory are NOT hidden in this envelope; snapshot owners drop before factory admission.
const TASK_BYTES: usize = 8 * 1024;
const TASK_AND_REPLY_BYTES: usize = 1024;

struct Worker {
    context: Context,
    route: Route,
    parent: ResolvedRow,
    purpose: ConfirmationPurpose,
    physical: Option<PhysicalHold>,
    payment: Credit,
    // Original affine Header/scratch must outlive all actual work, not a copied Session ID.
    incoming: Incoming,
}
struct Response {
    result: Result<(RowId, Status), Refused>,
    _payment: Credit,
    incoming: Incoming,
}
impl Response {
    fn seal(self) -> Result<CompletedReply, channel::Refused> {
        let Self {
            result,
            _payment,
            incoming,
        } = self;
        // Actual task/output has been collected; fixed physical Control pays serialization.
        drop(_payment);
        super::history::Response::new(incoming, result).seal()
    }
}

/// Read-only Begin(false) uses the existing history path with no task/snapshot admission.
/// First admission/retry uses ONE paid detached worker. Outer waiter/socket loss never drops
/// its snapshot, original physical request or possibly admitted child factory prematurely.
pub(super) async fn begin(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, channel::Refused> {
    let prepared = prepare(context, route, &incoming);
    let target = match prepared {
        Ok(None) => return super::confirmation_history::recover(context, route, incoming).seal(),
        Ok(Some(target)) => target,
        Err(error) => return super::history::Response::new(incoming, Err(error)).seal(),
    };
    let physical = incoming.hold_position()?;
    let payment = match route.selected().resources().charge(
        route.selected().binding().scope.group,
        TASK_BYTES,
        ResourceKind::Retained,
    ) {
        Ok(payment) => payment,
        Err(error) => return super::history::Response::new(incoming, Err(error.into())).seal(),
    };
    let task = run(Worker {
        context: context.clone(),
        route: route.clone(),
        parent: target.0,
        purpose: target.1,
        physical: Some(physical),
        payment,
        incoming,
    });
    if worker_layout(std::mem::size_of_val(&task)) > TASK_BYTES {
        // No task has been polled and no snapshot/Actor effect exists on local refusal.
        return Err(channel::Refused::Capacity);
    }
    tokio::spawn(task)
        .await
        .map_err(|_| channel::Refused::Binding)?
        .seal()
}

fn prepare(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<Option<(ResolvedRow, ConfirmationPurpose)>, Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::BeginDerive
        || incoming.kind() != BodyKind::Control
        || incoming.logical() != [0; 16]
    {
        return Err(VoiceError::Unauthorized.into());
    }
    let request = Request::decode(
        incoming.operation(),
        incoming.kind(),
        incoming.logical(),
        incoming.plain().map_err(|_| VoiceError::Malformed)?,
    )?;
    let Kind::Begin { retry } = request.kind else {
        return Err(VoiceError::Malformed.into());
    };
    if !retry {
        return Ok(None);
    }
    let context = context.upgrade().ok_or(Refused::Closed)?;
    let parent = context.ledger.resolve(
        route.selected(),
        Purpose::Derive(request.selector),
        request.parent,
    )?;
    {
        let authority = context.authority.lock().unwrap_or_else(|e| e.into_inner());
        super::factory::require_current_selection(context.gate(), &authority, route.selected())?;
        route.require_open().map_err(|_| Refused::Closed)?;
    }
    Ok(Some((parent, request.purpose)))
}

async fn run(mut worker: Worker) -> Response {
    let result = create(&mut worker).await;
    let Worker {
        payment, incoming, ..
    } = worker;
    Response {
        result,
        _payment: payment,
        incoming,
    }
}
async fn create(worker: &mut Worker) -> Result<(RowId, Status), Refused> {
    let id = if let ConfirmationPurpose::Incoming(role) = worker.purpose {
        let known = {
            let context = worker.context.upgrade().ok_or(Refused::Closed)?;
            context
                .ledger
                .lookup_child(
                    worker.route.selected(),
                    &worker.parent,
                    worker.purpose,
                    None,
                )
                .ok()
        };
        let known = if let Some(child) = known {
            let status = child.status()?;
            if child.has_pending_publication()?
                || status.action.is_some()
                || matches!(status.pending, super::ledger::PendingStatus::Closed { .. })
            {
                Some(child)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(child) = known {
            super::factory::ensure_known_incoming_row(
                &worker.context,
                &worker.route,
                &worker.parent,
                &child,
                worker.purpose,
                worker.physical.take().expect("one original position"),
            )
            .await?
        } else {
            let action =
                observe_incoming(&worker.context, &worker.route, &worker.parent, role).await?;
            // Rechecks current UI generation/selection/route AFTER the read-only snapshot.
            // A delayed query may not adopt a stale action into a replacement call.
            super::factory::ensure_incoming_row(
                &worker.context,
                &worker.route,
                &worker.parent,
                action,
                worker.physical.take().expect("one original position"),
            )
            .await?
        }
    } else {
        super::factory::ensure_confirmation_row(
            &worker.context,
            &worker.route,
            &worker.parent,
            worker.purpose,
            worker.physical.take().expect("one original position"),
        )
        .await?
    };
    let context = worker.context.upgrade().ok_or(Refused::Closed)?;
    let row = context.ledger.lookup_child(
        worker.route.selected(),
        &worker.parent,
        worker.purpose,
        Some(id),
    )?;
    Ok((id, row.reconcile()?))
}

/// ONE bounded native observation per request. Snapshot owns the actual32-slot backing Vec
/// and mailbox/output payment; select at most ONE original Arc, then drop the real snapshot
/// and its permit BEFORE a factory borrows metadata or a child Phase. No UI commit spans
/// this read-only Actor wait. The detached Worker retains Incoming/physical custody instead.
async fn observe_incoming(
    context: &Context,
    route: &Route,
    parent: &ResolvedRow,
    role: u8,
) -> Result<Arc<CryptoAction>, Refused> {
    let context = context.upgrade().ok_or(Refused::Closed)?;
    {
        let authority = context.authority.lock().unwrap_or_else(|e| e.into_inner());
        super::factory::require_current_selection(context.gate(), &authority, route.selected())?;
        route.require_open().map_err(|_| Refused::Closed)?;
    }
    let actor = {
        let servers = context.servers.lock().await;
        let b = route.selected().binding();
        servers
            .get(&b.server)
            .filter(|entry| entry.instance == b.instance && entry.group_id == b.group)
            .map(|entry| entry.actor.clone())
            .ok_or(Refused::Closed)?
    };
    let grant = route.selected().grant()?;
    let permit = CryptoActionSnapshotPermit::new(
        &route.selected().resources(),
        route.selected().binding().scope,
    )?;
    drop(context); // never detach a mounted store or broad strong authorization context
    let snapshot = actor.voice_confirmation_actions(grant, permit).await?;
    let mut found = None;
    for action in snapshot.result()? {
        if action.operation() == CryptoOperation::DecryptConfirmation
            && action.protected_row()? == role
            && parent.contains_plan_action(action)?
        {
            found = Some(action.clone());
            break;
        }
    }
    drop(snapshot); // destroys actual Vec/permit before returning the single actual owner
    found.ok_or(Refused::Protocol(VoiceError::Capacity))
}

fn worker_layout(future: usize) -> usize {
    future
        + std::mem::size_of::<Response>()
        + std::mem::size_of::<tokio::task::JoinHandle<Response>>()
        + 2 * std::mem::size_of::<usize>()
        // create/known-child selection performs synchronous native/Root reconciliation
        // between awaits. These locals are not necessarily stored in the compiler future.
        + std::mem::size_of::<super::context::LiveContext>()
        + 2 * std::mem::size_of::<ResolvedRow>()
        + std::mem::size_of::<RowId>()
        + super::ledger::reconciliation_layout_bytes()
        + 128
        + TASK_AND_REPLY_BYTES
}
#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    fn size<F: std::future::Future>(
        _: impl FnOnce(&'static Context, &'static Route, Incoming) -> F,
    ) -> usize {
        std::mem::size_of::<F>() + std::mem::size_of::<F::Output>()
    }
    let decode = std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<Request<'static>>()
        + std::mem::size_of::<ResolvedRow>()
        + 128;
    let serialization =
        std::mem::size_of::<Response>() + super::history::serialization_layout_bytes();
    // prepare's borrowing decoder has returned before a worker can be spawned. The worker
    // and its actual output are collected before serialization drops its separate payment;
    // count the whole synchronous serialization graph under Control at that later boundary.
    size(begin)
        + decode
            .max(serialization)
            .max(super::confirmation_history::layout_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_begin_worker_fits_its_separate_original_task_payment() {
        fn size<F: std::future::Future>(_: impl FnOnce(Worker) -> F) -> usize {
            worker_layout(std::mem::size_of::<F>())
        }
        let bytes = size(run);
        assert!(bytes <= TASK_BYTES, "Begin worker requires {bytes}");
    }

    #[tokio::test]
    async fn lost_start_begin_waiter_does_not_pin_mounted_context_at_factory_ui_wait() {
        use super::super::derivation_result::tests::Case;
        use super::super::service::tests::opened_frame;
        use std::future::{poll_fn, Future};
        use std::task::Poll;
        let case = Case::new().await;
        let before = case.selected.resources().counts();
        let boundary = case.native.state.ui_session_commit.lock().await;
        let commit_owners = Arc::strong_count(&case.native.state.ui_session_commit);
        let store_owners = Arc::strong_count(&case.native.state.store);
        let mut body = vec![2];
        body.extend_from_slice(case.selector.peer.as_bytes());
        body.extend_from_slice(&case.selector.join);
        body.extend_from_slice(&case.selector.key_ref);
        body.extend_from_slice(&case.selector.epoch.to_be_bytes());
        body.extend_from_slice(&case.id.logical());
        body.extend_from_slice(&[0, 1]); // actual Start(None), Begin(true)
        let incoming = opened_frame(&case.public, &case.route, (1, 8, 0, [0; 16]), &body, None);
        let mut pending = Box::pin(begin(&case.context, &case.route, incoming));
        assert!(poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx).is_pending())).await);
        drop(pending); // actual framed caller loss, not cancellation of its paid worker
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            // Observe the real original commit handle after factory binding completes and
            // LiveContext drops. The old implementation pins store while this mutex waits.
            while Arc::strong_count(&case.native.state.ui_session_commit) <= commit_owners
                || Arc::strong_count(&case.native.state.store) != store_owners
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(case.route.session().fixture_counts().0);
        assert_eq!(case.selected.resources().counts().0, before.0 + TASK_BYTES);
        crate::register_ui_lock_request(&case.native.state);
        drop(boundary);
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            case.route.session().fixture_wait_physical_idle(),
        )
        .await
        .unwrap();
        assert!(case.native.state.ui_session_commit.try_lock().is_ok());
        assert!(case
            .native
            .state
            .voice_handoff
            .lookup_child(
                &case.selected,
                &case.row(),
                ConfirmationPurpose::Start(None),
                None
            )
            .is_err());
        assert!(case.action.terminal_status().receipt.is_none());
        assert!(case.selected.resources().counts().0 <= before.0);
        crate::lock_session_inner(&case.native.state, None)
            .await
            .unwrap();
        case.retire();
        case.native.close().await;
    }

    #[tokio::test]
    async fn incoming_role_hint_without_actual_intake_cannot_create_a_child() {
        use super::super::derivation_result::tests::Case;
        use super::super::service::tests::{opened_frame, opened_reply};
        let case = Case::unadmitted().await;
        let mut prefix = vec![2];
        prefix.extend_from_slice(case.selector.peer.as_bytes());
        prefix.extend_from_slice(&case.selector.join);
        prefix.extend_from_slice(&case.selector.key_ref);
        prefix.extend_from_slice(&case.selector.epoch.to_be_bytes());
        prefix.extend_from_slice(&case.id.logical());
        prefix.extend_from_slice(&[3, 1, 1]); // Incoming(1), Begin(true)
        let before = case.selected.resources().counts();
        for sequence in 1..=3 {
            let header = (sequence, 8, 0, [0; 16]);
            let reply = begin(
                &case.context,
                &case.route,
                opened_frame(&case.public, &case.route, header, &prefix, None),
            )
            .await
            .unwrap();
            assert_eq!(
                opened_reply(&case.public, header, &prefix, &reply),
                super::super::replies::refused(Refused::Protocol(VoiceError::Unauthorized))
            );
            // Real Actor rejects the fixture's synthetic grant. No role number supplies
            // a genuine Action/token and no empty native Incoming purpose is reserved.
            assert!(case
                .native
                .state
                .voice_handoff
                .lookup_child(
                    &case.selected,
                    &case.row(),
                    ConfirmationPurpose::Incoming(1),
                    None
                )
                .is_err());
            assert_eq!(case.selected.resources().counts(), before);
        }
        case.retire();
        case.native.close().await;
    }

    #[tokio::test]
    async fn lost_begin_waiter_keeps_actual_snapshot_and_physical_custody_until_collection() {
        use super::super::derivation_result::tests::Case;
        use super::super::factory::tests::park;
        use super::super::service::tests::opened_frame;
        use std::future::{poll_fn, Future};
        use std::task::Poll;
        for stop_first in [false, true] {
            let case = Case::unadmitted().await;
            let mut body = vec![2];
            body.extend_from_slice(case.selector.peer.as_bytes());
            body.extend_from_slice(&case.selector.join);
            body.extend_from_slice(&case.selector.key_ref);
            body.extend_from_slice(&case.selector.epoch.to_be_bytes());
            body.extend_from_slice(&case.id.logical());
            body.extend_from_slice(&[3, 1, 1]);
            let before = case.selected.resources().counts();
            let store_owners = Arc::strong_count(&case.native.state.store);
            let parked = park(&case.native.actor).await;
            if stop_first {
                case.native.actor.shutdown().await;
            }
            let incoming = opened_frame(&case.public, &case.route, (1, 8, 0, [0; 16]), &body, None);
            let mut pending = Box::pin(begin(&case.context, &case.route, incoming));
            assert!(poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx).is_pending())).await);
            drop(pending); // actual outer handler loss; original worker remains detached
            tokio::time::timeout(std::time::Duration::from_secs(15), async {
                while case.selected.resources().counts().0
                    < before.0 + TASK_BYTES + CryptoActionSnapshotPermit::MEMORY_BYTES
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert_eq!(
                Arc::strong_count(&case.native.state.store),
                store_owners,
                "snapshot wait retains only weak context, not a mounted store"
            );
            assert!(
                case.native.state.ui_session_commit.try_lock().is_ok(),
                "no UI commit around read-only Actor wait"
            );
            assert!(case.route.session().header().is_err());
            let mut idle = Box::pin(case.route.session().fixture_wait_physical_idle());
            assert!(poll_fn(|cx| Poll::Ready(idle.as_mut().poll(cx).is_pending())).await);
            drop(parked);
            tokio::time::timeout(std::time::Duration::from_secs(15), idle)
                .await
                .unwrap();
            assert_eq!(
                case.selected.resources().counts(),
                before,
                "actual snapshot/worker/output were collected, not merely abandoned by caller"
            );
            assert!(case
                .native
                .state
                .voice_handoff
                .lookup_child(
                    &case.selected,
                    &case.row(),
                    ConfirmationPurpose::Incoming(1),
                    None
                )
                .is_err());
            case.retire();
            case.native.close().await;
        }
    }
}
