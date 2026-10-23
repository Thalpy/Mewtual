//! Inactive typed executor for an original confirmation child result. It owns the actual authenticated request
//! through actor publication and completed-output collection, including after socket loss.
//! This effect executor is not exposed by the listener: ingress serves only Status and read-only history.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, Operation};
use super::context::Context;
use super::custody::PublicationCustody;
use super::factory::Refused;
use super::ledger::{Status, Work};
use super::requests::{confirmation::Request, Action, Kind};
use catcoms_app::{ServerActor, VoiceConfirmationBody};
use catcoms_sync::voice_v2::crypto_actions::CryptoOperation;
use catcoms_sync::voice_v2::{CallGrant, VoiceError};
use sha2::{Digest, Sha256};
use std::ops::Range;
use std::sync::Arc;

const TASK_BYTES: usize = 8 * 1024;
// Pinned Tokio task/oneshot/envelope overhead, in addition to concrete future/output/command
// layouts below. Incoming remains inline: count BOTH future and completed-output storage.
// Its ONE fixed scratch backing allocation stays explicitly paid/counted in Session control;
// moving the scratch handle cannot reserve duplicate4KiB arrays in each async state. This narrow
// child executor additionally borrows the SAME Phase for the separate4KiB actor body bridge;
// TASK_BYTES does not pay that bridge or its actual Data buffer twice.
const TASK_AND_REPLY_BYTES: usize = 1024;

/// Keep Work/task custody BEFORE Incoming in drop order. Its last physical Header must not
/// reopen the Session or wake shutdown while actual result-buffer/work owners still exist.
pub(super) struct Response {
    result: Result<Status, Refused>,
    work: Option<Work>,
    payment: Option<Arc<PublicationCustody>>,
    incoming: Incoming,
}
impl Response {
    /// The dispatcher collects the actual task output before building its one authenticated
    /// Control reply. Historical/refusal responses keep their original Data buffer; an
    /// admitted worker has transferred and collected it through the actual actor body.
    /// Incoming still owns the same physical header/response nonce. Fixed result metadata
    /// never proves that independent browser/Input/Phase owners have drained.
    pub(super) fn seal(self) -> Result<CompletedReply, channel::Refused> {
        let Self {
            result,
            work,
            payment,
            incoming,
        } = self;
        let result = result.and_then(|status| {
            let id = super::ledger::RowId::from_logical(incoming.logical())?;
            Ok((id, status))
        });
        // Keep the actual Work/publication payment through synchronous serialization too.
        // Historical/refusal replies have no task payment and use fixed physical Control.
        let reply = super::history::Response::new(incoming, result).seal();
        drop(work);
        drop(payment);
        reply
    }
}

/// Resolve the original native row from the decrypted request, never from browser identity
/// labels alone. The caller must supply an already opened frame and its actual native Route.
/// Historical recovery admits no Work, ADDITIONAL Phase contribution, task or body allocation.
/// It still owns the original Data body already admitted and paid by the physical channel.
pub(super) async fn commit(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<Response, Refused> {
    // Keep synchronous historical/decoder variants out of the async frame entirely.
    // Only the actual worker JoinHandle survives this scope; no historical Response is
    // retained alongside the task/output wait. layout_bytes also counts launch's graph.
    let task = match launch(context, route, incoming)? {
        Started::Immediate(response) => return Ok(response),
        Started::Task(task) => task,
    };
    task.await.map_err(|_| Refused::Stopped)
}
fn launch(context: &Context, route: &Route, incoming: Incoming) -> Result<Started, Refused> {
    // Resolve terminal history synchronously. A Status is much larger than the live action
    // selector; carrying it as prepare's async output embeds that unused branch throughout
    // every UI/Actor wait. No historical status or borrowing decoder crosses this scope.
    let target = match identify(context, route, &incoming) {
        Err(error) => {
            return Ok(Started::Immediate(Response {
                result: Err(error),
                work: None,
                payment: None,
                incoming,
            }))
        }
        Ok(Initial::Historical(status)) => {
            return Ok(Started::Immediate(Response {
                result: Ok(status),
                work: None,
                payment: None,
                incoming,
            }))
        }
        Ok(Initial::Work(target)) => target,
    };
    start(context, route, target, incoming)
}

// Construct the whole asynchronous preparation/publication worker only AFTER borrowing its
// original8KiB Data contribution. In particular, its UI/Actor waits and completed refusals
// must not occupy an additional uncharged Control future. No actor effect is admitted here.
enum Started {
    Immediate(Response),
    Task(tokio::task::JoinHandle<Response>),
}
fn start(
    context: &Context,
    route: &Route,
    target: Target,
    incoming: Incoming,
) -> Result<Started, Refused> {
    let work = match target.original.work() {
        Ok(work) => work,
        Err(error) => {
            return Ok(Started::Immediate(Response {
                result: Err(error.into()),
                work: None,
                payment: None,
                incoming,
            }))
        }
    };
    let payment = match work.phase().copy(TASK_BYTES) {
        Ok(payment) => payment,
        Err(error) => {
            return Ok(Started::Immediate(Response {
                result: Err(error.into()),
                work: Some(work),
                payment: None,
                incoming,
            }))
        }
    };
    let physical = incoming.hold_position().map_err(|_| Refused::Closed)?;
    let publication = Arc::new(PublicationCustody::preparing(physical, payment));
    let Target {
        original,
        tag,
        output_range,
    } = target;
    drop(original); // the SAME Work now owns its actual indexed row; no extra resolver needed
    let task = run(Worker {
        context: Some(context.clone()),
        route: Some(route.clone()),
        work,
        tag,
        output: output_range,
        publication: Some(publication),
        incoming,
    });
    #[cfg(test)]
    eprintln!(
        "confirmation result peak {}; future {}; response {}; admitted {}; command {}",
        worker_layout(&task),
        std::mem::size_of_val(&task),
        std::mem::size_of::<Response>(),
        std::mem::size_of::<Admitted>(),
        std::mem::size_of::<catcoms_app::AppCommand>()
    );
    if worker_layout(&task) > TASK_BYTES {
        // Proven local refusal: future was never polled and no actor request exists.
        // Dropping it releases original work/payment/request, never refunds a Root
        // publication or reports that an already admitted browser engine drained.
        return Err(Refused::Protocol(VoiceError::Capacity));
    }
    Ok(Started::Task(tokio::spawn(task)))
}
enum Initial {
    Historical(Status),
    Work(Target),
}
struct Target {
    original: super::ledger::ResolvedRow,
    tag: [u8; 32],
    output_range: Range<usize>,
}
// This value stays inline in the already-paid task. No ordinary-pool allocation is needed
// to prepare or publish an original admitted result; its commit joins the same custody.
struct Admitted {
    actor: ServerActor,
    grant: Arc<CallGrant>,
    commit: tokio::sync::OwnedMutexGuard<()>,
}
/// Declaration order applies even on unwind/runtime teardown: the actual work, grant/commit
/// and paid task contribution drop BEFORE Incoming's Header can clear its physical position.
struct Worker {
    context: Option<Context>,
    route: Option<Route>,
    work: Work,
    tag: [u8; 32],
    output: Range<usize>,
    publication: Option<Arc<PublicationCustody>>,
    incoming: Incoming,
}
fn identify(context: &Context, route: &Route, incoming: &Incoming) -> Result<Initial, Refused> {
    let context = context.upgrade().ok_or(Refused::Closed)?;
    let (original, identity, tag, output_range) = decode_result(&context, route, incoming)?;
    {
        // Scope fixed history metadata before first-effect awaits: admission needs only
        // the original action identity, not a retained copy of the full status graph.
        let status = original.reconcile()?;
        if status.action != Some((identity.serial, identity.version, identity.input_tag)) {
            return Err(Refused::Protocol(VoiceError::Unauthorized));
        }
        if let Some(receipt) = &status.engine.receipt {
            if receipt.output_tag != tag {
                return Err(Refused::Protocol(VoiceError::RetryMismatch));
            }
            // History is past metadata, with no current grant/keys/RTC authority.
            return Ok(Initial::Historical(status));
        }
    }
    Ok(Initial::Work(Target {
        original,
        tag,
        output_range,
    }))
}
async fn prepare(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
    work: &Work,
    tag: [u8; 32],
    output_range: Range<usize>,
) -> Result<Admitted, Refused> {
    let selected = route.selected();
    let commit = super::factory::preflight_binding(context, route).await?;
    let commit = commit.lock_owned().await;
    let context = context.upgrade().ok_or(Refused::Closed)?;
    context
        .gate()
        .require_unlocked()
        .await
        .map_err(|_| Refused::Closed)?;
    let actor = {
        let servers = context.servers.lock().await;
        let binding = selected.binding();
        servers
            .get(&binding.server)
            .filter(|entry| entry.instance == binding.instance && entry.group_id == binding.group)
            .map(|entry| entry.actor.clone())
            .ok_or(Refused::Closed)?
    };
    {
        let authority = context
            .authority
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        super::factory::require_current_selection(context.gate(), &authority, selected)?;
        route.require_open().map_err(|_| Refused::Closed)?;
    }
    let grant = selected.grant()?;
    let output = &incoming.plain().map_err(|_| VoiceError::Malformed)?[output_range.clone()];
    if !matches!(
        work.action().operation(),
        CryptoOperation::EncryptConfirmation | CryptoOperation::DecryptConfirmation
    ) || work.action().output_tag(output)? != tag
    {
        return Err(VoiceError::RetryMismatch.into());
    }
    // Never detach the broad mounted-store/Coordinator/native context. The original actor,
    // affine grant, commit and paid work are sufficient; Root rechecks authority in its turn.
    drop(context);
    Ok(Admitted {
        actor,
        grant,
        commit,
    })
}

/// The full borrowing decoder and transient parent handle end BEFORE the first suspension.
/// No plaintext copy or request descriptor remains live alongside actor-body construction.
fn decode_result(
    context: &super::context::LiveContext,
    route: &Route,
    incoming: &Incoming,
) -> Result<(super::ledger::ResolvedRow, Action, [u8; 32], Range<usize>), Refused> {
    // A different Session's successfully opened frame is not this Route's authority.
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::Result
        || incoming.kind() != BodyKind::Data
    {
        return Err(Refused::Protocol(VoiceError::Unauthorized));
    }
    let request = Request::decode(
        incoming.operation(),
        incoming.kind(),
        incoming.logical(),
        incoming
            .plain()
            .map_err(|_| Refused::Protocol(VoiceError::Malformed))?,
    )?;
    let Kind::Result {
        action: identity,
        tag,
        output,
    } = request.kind
    else {
        return Err(Refused::Protocol(VoiceError::Malformed));
    };
    if tag != <[u8; 32]>::from(Sha256::digest(output)) {
        return Err(Refused::Protocol(VoiceError::RetryMismatch));
    }
    let original = super::confirmation_input::resolve(context, route, &request)?;
    // The canonical result is the exact trailing slice; no pointer arithmetic or copy.
    let end = incoming.plain().map_err(|_| VoiceError::Malformed)?.len();
    let output_range = end - output.len()..end;
    Ok((original, identity, tag, output_range))
}
fn worker_layout<F>(future: &F) -> usize {
    worker_layout_bytes(std::mem::size_of_val(future))
}
fn worker_layout_bytes(future: usize) -> usize {
    future
        + std::mem::size_of::<Response>()
        + std::mem::size_of::<Admitted>()
        + std::mem::size_of::<tokio::task::JoinHandle<Response>>()
        + std::mem::size_of::<catcoms_app::AppCommand>()
        + std::mem::size_of::<PublicationCustody>()
        + 2 * std::mem::size_of::<usize>()
        // Preparation/hash/refresh now execute INSIDE this paid worker. Count their
        // synchronous borrowed context/digest/status/argument graph too, not just the
        // compiler's async frame or previously Control-paid decode/serialization.
        + std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<sha2::Sha256>()
        + std::mem::size_of::<Status>()
        + 32 + 128
        + TASK_AND_REPLY_BYTES
}

/// Concrete Control portion only. The detached worker, its shared publication allocation,
/// and actor body bridge are independently paid by8KiB+4KiB of the ORIGINAL16KiB Data Phase.
/// They must not be hidden here as an uncharged Box or additional ordinary-pool reservation.
#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    fn future<F: std::future::Future>(
        _: impl FnOnce(&'static Context, &'static Route, Incoming) -> F,
    ) -> usize {
        std::mem::size_of::<F>() + std::mem::size_of::<F::Output>()
    }
    let decode = std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<Request<'static>>()
        + 2 * std::mem::size_of::<super::ledger::ResolvedRow>()
        + std::mem::size_of::<sha2::Sha256>()
        + 32
        + std::mem::size_of::<Action>()
        + std::mem::size_of::<Range<usize>>()
        + 128;
    let start = std::mem::size_of::<Target>()
        + std::mem::size_of::<Work>()
        + std::mem::size_of::<super::custody::CopyCustody>()
        + std::mem::size_of::<super::channel::PhysicalHold>()
        + 128;
    // launch's historical/started alternatives are scoped before the first await. Include
    // BOTH its whole output and Initial history/target while constructing that output.
    let launch = std::mem::size_of::<Initial>() + std::mem::size_of::<Started>() + start;
    // The borrowing decoder returns before any await; asynchronous validation belongs to
    // the separately paid worker. Response::seal runs after task/output collection. Include
    // the complete synchronous reply graph explicitly, even without a worker payment.
    eprintln!(
        "result Control future/output {}; decode {}; launch {}; serialize {}",
        future(commit),
        decode,
        launch,
        super::history::serialization_layout_bytes()
    );
    future(commit)
        + decode
            .max(launch)
            .max(super::history::serialization_layout_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concrete_worker_and_publication_fit_the_original_task_contribution() {
        fn size<F: std::future::Future>(_: impl FnOnce(Worker) -> F) -> usize {
            worker_layout_bytes(std::mem::size_of::<F>())
        }
        let bytes = size(run);
        assert!(bytes <= TASK_BYTES, "result worker requires {bytes}");
        // Worst admitted Data body already includes its AEAD tag. Actor metadata pays its
        // bounded ResultOwner/Bytes/BodyState/flight/mailbox/completion graph; Publication's
        // pointee and Arc header are separately counted in worker_layout_bytes above.
        let total = bytes + catcoms_app::VOICE_CONFIRMATION_BODY_METADATA_BYTES + 2064;
        assert!(
            total <= 16 * 1024,
            "result worker/body/frame requires {total}"
        );
    }

    /// Exercises this actual paid worker, not the child decoder: the native Derive fixture
    /// deliberately cannot publish a confirmation result. Lock must refuse before that
    /// operation check, while the uncollected failure still pays its real task/physical lane.
    #[tokio::test]
    async fn caller_loss_during_binding_query_keeps_paid_worker_without_mounted_context() {
        use super::super::derivation_result::tests::Case;
        use super::super::factory::tests::park;
        use std::future::{poll_fn, Future};
        use std::task::Poll;
        let case = Case::new().await;
        let phase = case.row().work().unwrap().phase().clone();
        let incoming = case.frame(1, &case.bytes());
        let work = case.row().work().unwrap();
        let payment = work.phase().copy(TASK_BYTES).unwrap();
        let physical = incoming.hold_position().unwrap();
        let parked = park(&case.native.actor).await;
        let store_owners = Arc::strong_count(&case.native.state.store);
        let server_owners = Arc::strong_count(&case.native.state.servers);
        let mut worker = Box::pin(run(Worker {
            context: Some(case.context.clone()),
            route: Some(case.route.clone()),
            work,
            tag: case.action.output_tag(&[]).unwrap(),
            output: 187..187,
            publication: Some(Arc::new(PublicationCustody::preparing(physical, payment))),
            incoming,
        }));
        assert!(poll_fn(|cx| Poll::Ready(worker.as_mut().poll(cx).is_pending())).await);
        assert_eq!(Arc::strong_count(&case.native.state.store), store_owners);
        assert_eq!(Arc::strong_count(&case.native.state.servers), server_owners);
        assert!(case.route.session().fixture_counts().0);
        assert_eq!(phase.copy(TASK_BYTES).err(), Some(VoiceError::Capacity));
        let caller = tokio::spawn(worker);
        drop(caller); // actual detached worker, never a fabricated completed/drained bit
        crate::register_ui_lock_request(&case.native.state);
        drop(parked);
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            case.route.session().fixture_wait_physical_idle(),
        )
        .await
        .unwrap();
        assert!(case.action.terminal_status().receipt.is_none());
        drop(phase.copy(16 * 1024).unwrap());
        crate::lock_session_inner(&case.native.state, None)
            .await
            .unwrap();
        case.retire();
        case.native.close().await;
    }

    #[tokio::test]
    async fn pre_admission_refusal_keeps_actual_task_payment_and_position_until_collection() {
        use super::super::derivation_result::tests::Case;
        use std::future::{poll_fn, Future};
        use std::task::Poll;
        let case = Case::new().await;
        let phase = case.row().work().unwrap().phase().clone();
        let incoming = case.frame(1, &case.bytes());
        let work = case.row().work().unwrap();
        let payment = work.phase().copy(TASK_BYTES).unwrap();
        let physical = incoming.hold_position().unwrap();
        let publication = Arc::new(PublicationCustody::preparing(physical, payment));
        let boundary = case.native.state.ui_session_commit.lock().await;
        let mut worker = Box::pin(run(Worker {
            context: Some(case.context.clone()),
            route: Some(case.route.clone()),
            work,
            tag: case.action.output_tag(&[]).unwrap(),
            output: 187..187,
            publication: Some(publication),
            incoming,
        }));
        assert!(poll_fn(|cx| Poll::Ready(worker.as_mut().poll(cx).is_pending())).await);
        case.native.actor.member_count().await; // actual earlier binding query has completed
        assert!(poll_fn(|cx| Poll::Ready(worker.as_mut().poll(cx).is_pending())).await);
        // Waiting owns only the original commit handle, not a strong mounted-store context.
        assert_eq!(Arc::strong_count(&case.native.state.store), 1);
        crate::register_ui_lock_request(&case.native.state);
        drop(boundary);
        let task = tokio::spawn(worker); // the actual task is independent of a socket waiter
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            while !task.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(phase.copy(TASK_BYTES).err(), Some(VoiceError::Capacity));
        assert!(case.route.session().fixture_counts().0);
        assert!(case.action.terminal_status().receipt.is_none());
        let response = task.await.unwrap();
        assert!(matches!(response.result, Err(Refused::Closed)));
        assert!(response.work.is_some() && response.payment.is_some());
        assert_eq!(phase.copy(TASK_BYTES).err(), Some(VoiceError::Capacity));
        assert!(case.route.session().fixture_counts().0);
        drop(response);
        assert!(!case.route.session().fixture_counts().0);
        drop(phase.copy(16 * 1024).unwrap());
        crate::lock_session_inner(&case.native.state, None)
            .await
            .unwrap();
        case.retire();
        case.native.close().await;
    }

    #[tokio::test]
    async fn cancelling_early_worker_cannot_clear_physical_position_before_actual_work_retires() {
        use super::super::derivation_result::tests::Case;
        use std::future::{poll_fn, Future};
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        };
        use std::task::Poll;
        let case = Case::new().await;
        let incoming = case.frame(1, &case.bytes());
        let row = case.row();
        let work = row.work().unwrap();
        let payment = work.phase().copy(TASK_BYTES).unwrap();
        let physical = incoming.hold_position().unwrap();
        let boundary = case.native.state.ui_session_commit.lock().await;
        let mut worker = Box::pin(run(Worker {
            context: Some(case.context.clone()),
            route: Some(case.route.clone()),
            work,
            tag: case.action.output_tag(&[]).unwrap(),
            output: 187..187,
            publication: Some(Arc::new(PublicationCustody::preparing(physical, payment))),
            incoming,
        }));
        assert!(poll_fn(|cx| Poll::Ready(worker.as_mut().poll(cx).is_pending())).await);
        case.native.actor.member_count().await;
        assert!(poll_fn(|cx| Poll::Ready(worker.as_mut().poll(cx).is_pending())).await);
        let guard = row.fixture_hold_work_retirement();
        let before = Arc::strong_count(&case.action);
        let cancel = AtomicBool::new(false);
        // Work::drop first releases its actual Action Arc, THEN takes this held Row State.
        // Observe that real destructor progress, rather than a sleep or a fabricated flag.
        let (observed, busy) = std::thread::scope(|scope| {
            let action = &case.action;
            let cancelled = &cancel;
            let (sent, received) = mpsc::channel();
            let observer = scope.spawn(move || {
                while Arc::strong_count(action) >= before && !cancelled.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                if !cancelled.load(Ordering::Acquire) {
                    let _ = sent.send(());
                }
            });
            let dropped = scope.spawn(move || drop(worker)); // actual future cancellation
            let observed = received.recv_timeout(std::time::Duration::from_secs(5));
            let busy = case.route.session().fixture_counts().0;
            cancel.store(true, Ordering::Release);
            drop(guard); // release BEFORE joins/assertions, including a failing regression
            dropped.join().unwrap();
            observer.join().unwrap();
            (observed, busy)
        });
        observed.unwrap();
        assert!(
            busy,
            "physical idle preceded actual blocked Work retirement"
        );
        assert!(!case.route.session().fixture_counts().0);
        assert!(case.action.terminal_status().receipt.is_none());
        drop(boundary);
        case.retire();
        case.native.close().await;
    }

    #[tokio::test]
    async fn renderer_result_labels_cannot_create_a_native_child_or_admit_actor_work() {
        use super::super::derivation_result::tests::Case;
        use super::super::service::tests::opened_frame;
        let case = Case::unadmitted().await;
        let foreign = Case::unadmitted().await;
        let mut row = case.id.logical();
        row[15] += 1; // canonical child-shaped header, with NO actual registered child
        let mut body = vec![2];
        body.extend_from_slice(case.selector.peer.as_bytes());
        body.extend_from_slice(&case.selector.join);
        body.extend_from_slice(&case.selector.key_ref);
        body.extend_from_slice(&case.selector.epoch.to_be_bytes());
        body.extend_from_slice(&case.id.logical());
        body.push(0); // Start(None)
        body.extend_from_slice(&case.action.serial().to_be_bytes());
        body.extend_from_slice(&case.action.version().to_be_bytes());
        body.extend_from_slice(&case.action.input_tag());
        body.extend_from_slice(&<[u8; 32]>::from(Sha256::digest([9; 16])));
        body.extend_from_slice(&16u16.to_be_bytes());
        body.extend_from_slice(&[9; 16]);
        let phase = case.row().work().unwrap().phase().clone();
        let before = case.selected.resources().counts();
        for (index, offset) in [
            None,
            Some(0),
            Some(1),
            Some(105),
            Some(121),
            Some(170),
            Some(204),
        ]
        .into_iter()
        .enumerate()
        {
            let mut bytes = body.clone();
            if let Some(offset) = offset {
                bytes[offset] ^= 1;
            }
            let incoming = opened_frame(
                &case.public,
                &case.route,
                (index as u64 + 1, 3, 1, row),
                &bytes,
                Some(&phase),
            );
            let response = commit(&case.context, &case.route, incoming).await.unwrap();
            assert!(response.result.is_err());
            assert!(response.work.is_none() && response.payment.is_none());
            assert_eq!(case.selected.resources().counts(), before);
            assert!(case.native.state.ui_session_commit.try_lock().is_ok());
            drop(response);
        }
        let foreign_phase = foreign.row().work().unwrap().phase().clone();
        let incoming = opened_frame(
            &foreign.public,
            &foreign.route,
            (1, 3, 1, row),
            &body,
            Some(&foreign_phase),
        );
        let response = commit(&case.context, &case.route, incoming).await.unwrap();
        assert!(matches!(
            response.result,
            Err(Refused::Protocol(VoiceError::Unauthorized))
        ));
        assert!(response.work.is_none() && response.payment.is_none());
        drop(response);
        assert!(case.action.terminal_status().receipt.is_none());
        case.retire();
        foreign.retire();
        case.native.close().await;
        foreign.native.close().await;
    }
}
async fn run(mut worker: Worker) -> Response {
    // Preparation is part of this ALREADY paid detached task. Failure before UI admission
    // still retains its original payment/physical hold through actual output collection.
    let prepared = prepare(
        worker.context.as_ref().expect("original weak context"),
        worker.route.as_ref().expect("original route"),
        &worker.incoming,
        &worker.work,
        worker.tag,
        worker.output.clone(),
    )
    .await;
    // Keep lifecycle owners in their ORDERED aggregate across every suspension/unwind.
    // Destructuring into locals here would drop Incoming before Work on cancellation.
    // Only optional resolution context/Route may leave early, before the Actor wait.
    drop(worker.context.take());
    drop(worker.route.take());
    let result = match prepared {
        Err(error) => Err(error),
        Ok(admitted) => {
            Arc::get_mut(worker.publication.as_mut().expect("original task custody"))
                .expect("no body/publication clone before native admission")
                .admit(admitted.commit);
            // The same actual body allocation is restored on local metadata refusal.
            // Only successful transfer lends a custody clone to independently queued work.
            match worker.incoming.result_body_with_publication(
                worker.work.phase(),
                worker.output.clone(),
                worker.publication.clone(),
            ) {
                Err(error) => Err(Refused::Protocol(match error {
                    super::channel::Refused::Capacity => VoiceError::Capacity,
                    _ => VoiceError::Malformed,
                })),
                Ok(body) => {
                    publish(
                        &worker.work,
                        admitted.actor,
                        admitted.grant,
                        &body,
                        worker.tag,
                    )
                    .await
                }
            } // body drops before the completed output can become the last payment owner
        }
    };
    Response {
        result,
        work: Some(worker.work),
        payment: worker.publication,
        incoming: worker.incoming,
    }
}

/// Only actual admitted UI commit custody can reach this Actor call. Caller/task loss cannot
/// release that commit ahead of queued bytes, and Root repeats its own effect authorization.
async fn publish(
    work: &Work,
    actor: ServerActor,
    grant: Arc<CallGrant>,
    body: &VoiceConfirmationBody,
    tag: [u8; 32],
) -> Result<Status, Refused> {
    let submitted = match work.action().operation() {
        CryptoOperation::EncryptConfirmation => {
            actor
                .submit_voice_confirmation_result(grant, work.action().clone(), body, tag)
                .await
        }
        CryptoOperation::DecryptConfirmation => {
            actor
                .commit_voice_confirmation_result(grant, work.action().clone(), body, tag)
                .await
        }
        CryptoOperation::Derive => {
            unreachable!("child operation checked before ownership transfer")
        }
    };
    match submitted {
        Ok(completed) => match completed.result() {
            Ok(_) => work.refresh().map_err(Refused::from),
            Err(error) => Err(error.into()),
        },
        Err(error) => Err(error.into()),
    } // the actual completed Actor reply is collected before body/publication collection
}
