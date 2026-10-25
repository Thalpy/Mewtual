//! Original native media admission. This owns metadata/callback custody only: the actual
//! browser peer must move affinely under independent payment after a joined fresh reply.
//! No early negotiation release, caller-supplied currentness, SDP string or new PC exists here.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, PhysicalHold};
use super::context::Context;
use super::factory::{self, Refused};
use super::ledger::{MediaIdentity, MediaReceipt, MediaRows, MediaWorker, ResolvedRow};
use super::requests::sdp_action::Begin;
use catcoms_app::{
    VoiceEndpointError, VoiceInitialMediaPublisher, VoiceInitialMediaWork,
    VOICE_INITIAL_MEDIA_RPC_BYTES,
};
use catcoms_sync::voice_v2::chain::sdp_handoff::{Copy, Phase, TaskCopy};
use catcoms_sync::voice_v2::sdp::{InitialSdpIntent, MediaAdmission, SdpMedia};
use catcoms_sync::voice_v2::VoiceError;
use std::sync::Arc;
use tokio::sync::oneshot;

// This distinct metadata task fits the ORIGINAL64KiB envelope after both SDP rows drain.
// It does not enlarge any process/group/phase/physical cap; layout below measures its whole
// native graph separately from the original4096B Actor RPC and detached8KiB media record.
const TASK_BYTES: usize = 4096;
struct Target {
    parent: ResolvedRow,
    original: Begin<'static>,
    identity: MediaIdentity,
    rows: MediaRows,
    phase: Arc<Phase>,
    intent: Arc<InitialSdpIntent>,
    media: Arc<SdpMedia>,
}
struct Custody {
    _physical: PhysicalHold,
    _worker: MediaWorker,
    // LAST: actual worker/physical destruction is paid even if an output is immediately
    // collected while its consuming sender is still returning from a native callback.
    _task: TaskCopy,
}
struct Publisher {
    context: Context,
    route: Route,
    target: Option<Box<Target>>,
    reply: Option<oneshot::Sender<Box<Completion>>>,
    // Fixture barrier owns NO custody. The actual entered Publisher must pay its return.
    #[cfg(test)]
    after_send: Option<tests::AfterSend>,
    custody: Arc<Custody>,
}
struct Completion {
    result: Result<MediaReceipt, Refused>,
    target: Box<Target>,
    _work: Option<VoiceInitialMediaWork>,
    _custody: Arc<Custody>,
}
struct Waiter {
    reply: oneshot::Receiver<Box<Completion>>,
    work: Option<VoiceInitialMediaWork>,
    _custody: Arc<Custody>,
}
impl Waiter {
    async fn complete(mut self) -> Result<Box<Completion>, Refused> {
        let output = (&mut self.reply).await.map_err(|_| Refused::Stopped)?;
        if let Some(work) = &self.work {
            work.wait_retired().await;
        }
        Ok(output)
    }
}
// Fixed preparation aggregation preserves payment/destructor order on every refusal or
// unwind. No owned native/Root graph is moved underneath UI/Coordinator/row/Group guards.
struct Construction {
    parent: Option<ResolvedRow>,
    original: Option<Begin<'static>>,
    identity: Option<MediaIdentity>,
    rows: Option<MediaRows>,
    phase: Option<Arc<Phase>>,
    intent: Option<Arc<InitialSdpIntent>>,
    media: Option<Arc<SdpMedia>>,
    rpc: Option<Copy>,
    worker: Option<MediaWorker>,
    physical: Option<PhysicalHold>,
    custody: Option<Arc<Custody>>,
    task: Option<TaskCopy>,
}
fn queue(
    context: &Context,
    route: &Route,
    parent: ResolvedRow,
    original: Begin<'static>,
    physical: PhysicalHold,
) -> Result<Waiter, Refused> {
    queue_impl(
        context,
        route,
        parent,
        original,
        physical,
        QueueOptions::default(),
    )
}
/// Shipping builds have no configurable authority. Fixtures exercise the same actual
/// Actor/Publisher path with an invalid grant or a barrier after reply delivery.
#[derive(Default)]
struct QueueOptions {
    #[cfg(test)]
    grant: Option<Arc<catcoms_sync::voice_v2::CallGrant>>,
    #[cfg(test)]
    after_send: Option<tests::AfterSend>,
}
fn queue_impl(
    context: &Context,
    route: &Route,
    parent: ResolvedRow,
    original: Begin<'static>,
    physical: PhysicalHold,
    _options: QueueOptions,
) -> Result<Waiter, Refused> {
    let mut c = Construction {
        parent: Some(parent),
        original: Some(original),
        identity: None,
        rows: None,
        phase: None,
        intent: None,
        media: None,
        rpc: None,
        worker: None,
        physical: Some(physical),
        custody: None,
        task: None,
    };
    let (phase, intent) = c
        .parent
        .as_ref()
        .unwrap()
        .media_original_parts(route.session().identity(), c.original.as_ref().unwrap())?;
    c.phase = Some(phase);
    c.intent = Some(intent);
    if task_layout() > TASK_BYTES
        || catcoms_app::voice_initial_media_rpc_bytes() > VOICE_INITIAL_MEDIA_RPC_BYTES
    {
        return Err(VoiceError::Capacity.into());
    }
    c.task = Some(c.phase.as_ref().unwrap().task_copy(TASK_BYTES)?);
    c.rpc = Some(
        c.phase
            .as_ref()
            .unwrap()
            .copy(VOICE_INITIAL_MEDIA_RPC_BYTES)?,
    );
    c.identity = Some(
        c.parent
            .as_ref()
            .unwrap()
            .media_identity(route.session().identity(), c.original.as_ref().unwrap())?,
    );
    let ledger = { context.upgrade().ok_or(Refused::Closed)?.ledger.clone() };
    c.rows = Some(ledger.media_rows(
        c.parent.as_ref().unwrap(),
        route.selected(),
        route.session().identity(),
        c.phase.as_ref().unwrap(),
    )?);
    let (wall, mono) = observe_time(context)?;
    let (worker, retained) = super::sdp::current(context, route, |admission| {
        c.parent.as_ref().unwrap().begin_media_worker(
            admission,
            c.original.as_ref().unwrap(),
            c.physical.as_ref().unwrap(),
            route.session(),
            c.identity.as_ref().unwrap(),
            wall,
            mono,
        )
    })?;
    c.worker = Some(worker);
    c.media = retained;
    if c.media.is_none() {
        let rows = c.rows.as_ref().unwrap();
        let prepared = c.intent.as_ref().unwrap().prepare_media(
            c.phase.as_ref().unwrap(),
            rows.seal(),
            rows.open(),
            wall,
            mono,
        )?;
        // No callback/await or try-only attachment may discard this irreversible child.
        // The worker stores the SAME inert capacity even when End/Close wins meanwhile.
        c.worker.as_ref().unwrap().attach(prepared.clone())?;
        c.media = Some(prepared);
    }
    c.custody = Some(Arc::new(Custody {
        _physical: c.physical.take().unwrap(),
        _worker: c.worker.take().unwrap(),
        _task: c.task.take().unwrap(),
    }));
    let (actor, grant) = factory::native::observation_actor(context, route)?;
    #[cfg(test)]
    let grant = _options.grant.unwrap_or(grant);
    let target = Box::new(Target {
        parent: c.parent.take().unwrap(),
        original: c.original.take().unwrap(),
        identity: c.identity.take().unwrap(),
        rows: c.rows.take().unwrap(),
        phase: c.phase.take().unwrap(),
        intent: c.intent.take().unwrap(),
        media: c.media.take().unwrap(),
    });
    let custody = c.custody.as_ref().unwrap().clone();
    let (reply, receiver) = oneshot::channel();
    let intent = target.intent.clone();
    let media = target.media.clone();
    let seal = target.rows.seal().clone();
    let open = target.rows.open().clone();
    let publisher = Box::new(Publisher {
        context: context.clone(),
        route: route.clone(),
        target: Some(target),
        reply: Some(reply),
        #[cfg(test)]
        after_send: _options.after_send,
        custody: custody.clone(),
    });
    let work = actor
        .try_queue_voice_initial_media(
            grant,
            intent,
            media,
            seal,
            open,
            c.rpc.take().unwrap(),
            publisher,
        )
        .ok();
    Ok(Waiter {
        reply: receiver,
        work,
        _custody: custody,
    })
}
impl Publisher {
    fn admit(&self, step: MediaAdmission<'_>) -> Result<MediaReceipt, Refused> {
        let target = self.target.as_ref().expect("original media target");
        super::sdp::current(&self.context, &self.route, |admission| {
            target.parent.commit_media(
                admission,
                &target.original,
                &target.identity,
                &target.rows,
                &target.phase,
                &target.intent,
                &target.media,
                step,
            )
        })
    }
    fn complete(
        &mut self,
        result: Result<MediaReceipt, Refused>,
        work: Option<VoiceInitialMediaWork>,
    ) {
        let output = Box::new(Completion {
            result,
            target: self.target.take().expect("original media target"),
            _work: work,
            _custody: self.custody.clone(),
        });
        let _ = self
            .reply
            .take()
            .expect("one original media reply")
            .send(output);
        // This borrowed helper returns BEFORE the consuming Publisher Box relinquishes its
        // independent custody. An immediate/dropped receiver cannot refund the entered sender.
        #[cfg(test)]
        if let Some(barrier) = self.after_send.take() {
            barrier.park();
        }
    }
}
impl VoiceInitialMediaPublisher for Publisher {
    fn publish(mut self: Box<Self>, step: MediaAdmission<'_>, work: VoiceInitialMediaWork) {
        let result = self.admit(step);
        self.complete(result, Some(work));
    }
    fn refused(mut self: Box<Self>, error: VoiceEndpointError) {
        self.complete(Err(error.into()), None);
    }
}

fn observe_time(context: &Context) -> Result<(u64, u64), Refused> {
    let clock = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        let authority = match live.authority.try_lock() {
            Ok(v) => v,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity.into()),
        };
        authority.runtime().clock()
    };
    Ok((clock.now_ms(), clock.monotonic_ms()))
}

enum Started {
    Immediate(Option<MediaReceipt>),
    Queued(Waiter),
}
/// All decoding/native preparation stays synchronous. Only the small actual waiter crosses
/// await; the physical Control future never retains another copy of the full preparation.
fn start(context: &Context, route: &Route, incoming: &Incoming) -> Result<Started, Refused> {
    use super::requests::media::{ExistingKind, OriginalKind, Request};
    if incoming.operation() != super::channel::Operation::Media
        || incoming.kind() != BodyKind::Control
        || !incoming.belongs_to(route.session())
    {
        return Err(VoiceError::Unauthorized.into());
    }
    let request = Request::decode(
        incoming.logical(),
        incoming.plain().map_err(|_| VoiceError::Malformed)?,
    )?;
    let parent_id = match &request {
        Request::Original { original, .. } => original.parent,
        Request::Existing { parent, .. } => *parent,
    };
    let parent = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        live.channels
            .require_cleanup_route(route)
            .map_err(|_| Refused::Closed)?;
        live.ledger
            .resolve_sdp_parent(route.selected(), parent_id)?
    };
    match request {
        Request::Original {
            original,
            kind: OriginalKind::Admit,
        } => {
            if let Some(history) = parent.media_original_history(
                route.session().identity(),
                &original,
                OriginalKind::Resolve,
            )? {
                if history.closed || history.released {
                    return Ok(Started::Immediate(Some(history)));
                }
                if history.admitted {
                    let (_, mono) = observe_time(context)?;
                    let current = super::sdp::current(context, route, |admission| {
                        parent.media_current(
                            admission,
                            original.pc.base.action,
                            history.proof,
                            mono,
                        )
                    })?;
                    return Ok(Started::Immediate(Some(current)));
                }
            }
            Ok(Started::Queued(queue(
                context,
                route,
                parent,
                original,
                incoming.hold_position().map_err(|_| VoiceError::Capacity)?,
            )?))
        }
        Request::Original { original, kind } => parent
            .media_original_history(route.session().identity(), &original, kind)
            .map(Started::Immediate)
            .map_err(Into::into),
        Request::Existing {
            action,
            proof,
            kind: ExistingKind::Current,
            ..
        } => {
            let (_, mono) = observe_time(context)?;
            super::sdp::current(context, route, |admission| {
                parent.media_current(admission, action, proof, mono)
            })
            .map(|r| Started::Immediate(Some(r)))
        }
        Request::Existing {
            action,
            proof,
            kind,
            ..
        } => parent
            .media_existing_history(route.session().identity(), action, proof, kind)
            .map(|r| Started::Immediate(Some(r)))
            .map_err(Into::into),
    }
}
fn collect(mut output: Box<Completion>) -> Result<MediaReceipt, Refused> {
    let result = std::mem::replace(&mut output.result, Err(Refused::Stopped));
    // Callback/Root retirement already joined. Actual target/work/custody destruction is
    // outside every admission guard, before fixed metadata serialization on Control credit.
    drop(output);
    result
}
const REPLY_BYTES: usize = 39;
pub(super) async fn apply(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, channel::Refused> {
    let result = match start(context, route, &incoming) {
        Ok(Started::Immediate(receipt)) => Ok(receipt),
        Ok(Started::Queued(waiter)) => waiter.complete().await.and_then(collect).map(Some),
        Err(error) => Err(error),
    };
    let length = match &result {
        Ok(Some(_)) => REPLY_BYTES,
        Ok(None) => 2,
        Err(_) => 4,
    };
    let mut reply = incoming.reply(BodyKind::Control, length, None)?;
    match result {
        Ok(Some(receipt)) => {
            let bytes = reply.plain_mut();
            bytes[0] = 9;
            bytes[1] = 0;
            bytes[2] = u8::from(receipt.admitted)
                | (u8::from(receipt.closed) << 1)
                | (u8::from(receipt.released) << 2)
                | (u8::from(receipt.current) << 3);
            bytes[3..35].copy_from_slice(&receipt.proof);
            bytes[35..39].copy_from_slice(&receipt.retire_in.unwrap_or(u32::MAX).to_be_bytes());
        }
        Ok(None) => reply.plain_mut().copy_from_slice(&[9, 1]),
        Err(error) => {
            let mut bytes = super::sdp_replies::refused(error);
            bytes[0] = 9;
            reply.plain_mut().copy_from_slice(&bytes);
        }
    }
    reply.seal()
}

#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    fn future<F>(_make: impl FnOnce(&'static Context, &'static Route, Incoming) -> F) -> usize {
        std::mem::size_of::<F>()
    }
    future(apply)
        + std::mem::size_of::<Started>()
        + std::mem::size_of::<super::requests::media::Request>()
        + std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<crate::UiSessionGate<'static>>()
        + 8 * std::mem::size_of::<Arc<()>>()
        + 6 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 512 // short selector/prepayment/current-origin staging, reply/result wrappers
}
fn task_layout() -> usize {
    // The complete concrete constructor/sender/receiver/entered-callback graph, plus the
    // Root preparation snapshots. Persistent rows/Phase/media records keep their OWN fees.
    std::mem::size_of::<Construction>()+std::mem::size_of::<QueueOptions>()+std::mem::size_of::<Target>()
        + std::mem::size_of::<Publisher>()+std::mem::size_of::<Completion>()+std::mem::size_of::<Waiter>()
        + std::mem::size_of::<(usize,usize,Custody)>()
        + 256 // conservative locked Tokio1.53.3 oneshot Inner (state/value/two task wakers/Arc)
        + std::mem::size_of::<crate::UiSessionGate<'static>>()
        + std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<super::transition::Admission<'static>>()
        + 8*std::mem::size_of::<std::sync::MutexGuard<'static,()>>()
        + 2*std::mem::size_of::<catcoms_sync::voice_v2::crypto_actions::CryptoHandoffStatus>()
        + std::mem::size_of::<catcoms_sync::voice_v2::resources::Credit>()
        + catcoms_sync::voice_v2::resources::Reservation::partition_metadata_bytes()
        + 512 // bounded constructor/hash/guard/result and synchronous Root preparation frames
}
#[cfg(test)]
pub(in crate::voice_handoff) mod tests;
