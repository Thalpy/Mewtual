//! Private SDP engine admission through the actual Actor. Native pays only this
//! queue/publication graph; the original Root Exchange owns engine credit through
//! actual SDK/result settlement. Authenticated service dispatch is enabled;
//! App V2 call selection remains disabled.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, PhysicalHold};
use super::context::Context;
use super::factory::{self, Refused};
use super::ledger::{ResolvedRow, SdpHistory, SdpInputTarget as Target, SdpSubmissionOwners};
use super::requests::sdp_action::{Kind, Request};
use super::requests::Action;
use super::sdp_publication::TASK_BYTES;
use catcoms_app::{
    VoiceEndpointError, VoiceSdpEnginePublisher, VoiceSdpEngineWork, VOICE_SDP_ENGINE_RPC_BYTES,
};
use catcoms_sync::voice_v2::chain::sdp_handoff::{Copy, TaskCopy};
use catcoms_sync::voice_v2::crypto_actions::SdpEngineRead;
use catcoms_sync::voice_v2::sdp::SdpEngineAdmission;
use catcoms_sync::voice_v2::VoiceError;
use std::sync::Arc;
use tokio::sync::oneshot;

struct Custody {
    _physical: PhysicalHold,
    // LAST: concrete sender/channel/output backing dies before this original payment.
    _task: TaskCopy,
}
struct Publisher {
    context: Context,
    route: Route,
    target: Option<Box<Target>>,
    reply: Option<oneshot::Sender<Box<Completion>>>,
    // LAST: an immediate receiver cannot refund this synchronous sender's return.
    custody: Arc<Custody>,
}
struct Completion {
    result: Result<SdpEngineRead, Refused>,
    target: Box<Target>,
    _work: Option<VoiceSdpEngineWork>,
    _custody: Arc<Custody>,
}
impl std::fmt::Debug for Completion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SdpEngineCompletion { .. }")
    }
}
struct Waiter {
    reply: oneshot::Receiver<Box<Completion>>,
    work: Option<VoiceSdpEngineWork>,
    _custody: Arc<Custody>,
}
impl Waiter {
    async fn complete(mut self) -> Result<Box<Completion>, Refused> {
        let output = (&mut self.reply).await.map_err(|_| Refused::Stopped)?;
        if let Some(work) = &self.work {
            // Root may still be dropping unused candidate credit after synchronous
            // send. History observes Clock only after the actual callback retires.
            work.wait_retired().await;
        }
        Ok(output)
    }
}
struct Construction {
    target: Option<Box<Target>>,
    owners: Option<SdpSubmissionOwners>,
    rpc: Option<Copy>,
    custody: Option<Arc<Custody>>,
    physical: Option<PhysicalHold>,
    task: Option<TaskCopy>,
}

fn queue(
    context: &Context,
    route: &Route,
    row: ResolvedRow,
    action: Action,
    engine_bytes: u32,
    physical: PhysicalHold,
) -> Result<Waiter, Refused> {
    let mut construction = Construction {
        target: None,
        owners: None,
        rpc: None,
        custody: None,
        physical: Some(physical),
        task: None,
    };
    if !construction
        .physical
        .as_ref()
        .expect("original request")
        .belongs_to(route.session())
    {
        return Err(VoiceError::Unauthorized.into());
    }
    if task_layout() > TASK_BYTES {
        return Err(VoiceError::Capacity.into());
    }
    construction.owners = Some(row.sdp_input_owners(route.session().identity(), action)?);
    let owners = construction.owners.as_ref().expect("original row funding");
    construction.task = Some(owners.phase.task_copy(TASK_BYTES)?);
    construction
        .task
        .as_mut()
        .expect("paid native graph")
        .bind_native_history(owners.funding.clone())?;
    construction.target = Some(Box::new(row.sdp_input_target(
        route.session().identity(),
        action,
        owners,
        construction.task.as_ref().expect("original Task"),
    )?));
    if construction
        .target
        .as_ref()
        .expect("original target")
        .action
        .engine_bytes()
        != engine_bytes as usize
    {
        return Err(VoiceError::RetryMismatch.into());
    }
    construction.rpc = Some(owners.phase.copy(VOICE_SDP_ENGINE_RPC_BYTES)?);
    construction
        .rpc
        .as_mut()
        .expect("paid Actor graph")
        .bind_native_history(owners.funding.clone())?;
    construction.custody = Some(Arc::new(Custody {
        _physical: construction.physical.take().expect("original request"),
        _task: construction.task.take().expect("original native payment"),
    }));
    let (actor, grant) = factory::native::observation_actor(context, route)?;
    let target = construction.target.take().expect("original target");
    let intent = target.intent.clone();
    let original = target.action.clone();
    let custody = construction
        .custody
        .as_ref()
        .expect("original custody")
        .clone();
    let (reply, receiver) = oneshot::channel();
    let publisher = Box::new(Publisher {
        context: context.clone(),
        route: route.clone(),
        target: Some(target),
        reply: Some(reply),
        custody: custody.clone(),
    });
    // The same consuming publisher synchronously sends any local refusal. Its
    // observer still owns original Task custody through collection of that output.
    let work = actor
        .try_queue_voice_sdp_engine(
            grant,
            intent,
            original,
            engine_bytes as usize,
            construction.rpc.take().expect("original Actor payment"),
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
    fn admit(&self, step: SdpEngineAdmission<'_>) -> Result<SdpEngineRead, Refused> {
        let live = self.context.upgrade().ok_or(Refused::Closed)?;
        let target = self.target.as_ref().expect("original target");
        // Verify BEFORE any engine effect: a same-intent Action supplied to App is
        // not necessarily this indexed native row's actual registered allocation.
        if !step.belongs_to_action(&target.action) {
            return Err(VoiceError::Unauthorized.into());
        }
        let result = {
            let commit = live
                .commit
                .clone()
                .try_lock_owned()
                .map_err(|_| VoiceError::Capacity)?;
            factory::require_unlocked_now(&live, self.route.selected())?;
            let authority = match live.authority.try_lock() {
                Ok(authority) => authority,
                Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity.into()),
            };
            let admission =
                super::transition::Admission::new(&live, &authority, &self.route, &commit);
            self.route
                .try_with_open(|| target.admit(&admission, || step.commit()))
        };
        // UI is LEXICAL, unlike Input Data custody. Root may destroy an unused
        // candidate after this hook returns; retaining UI in output would hide a
        // callback/credit destructor beneath native admission. Only actual original
        // engine credit remains retained by Engine through real SDK settlement.
        result.map_err(Into::into)
    }
    fn complete(
        &mut self,
        result: Result<SdpEngineRead, Refused>,
        work: Option<VoiceSdpEngineWork>,
    ) {
        let output = Box::new(Completion {
            result,
            target: self.target.take().expect("original target"),
            _work: work,
            _custody: self.custody.clone(),
        });
        let _ = self.reply.take().expect("one original reply").send(output);
        // The consuming trait callback retains its ACTUAL Publisher Box through
        // this borrowed helper's send and return, including an immediate receiver.
        // Its last custody pays failed send and that callback's eventual return.
    }
}
impl VoiceSdpEnginePublisher for Publisher {
    fn publish(mut self: Box<Self>, step: SdpEngineAdmission<'_>, work: VoiceSdpEngineWork) {
        let result = self.admit(step);
        self.complete(result, Some(work));
    }
    fn refused(mut self: Box<Self>, error: VoiceEndpointError) {
        self.complete(Err(error.into()), None);
    }
}

enum Prepared {
    History(Box<SdpHistory>),
    Completion(Box<Completion>),
}

/// Authenticated typed service handler. Original native/Root admission, rather than a
/// successful header or cached receipt, remains the final engine execution boundary.
pub(super) async fn admit(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, channel::Refused> {
    let result = prepare(context, route, &incoming).await;
    let projected = project(route, result);
    let encoded = serialize(projected);
    // Native Task/Target and receiver-side Actor references are destroyed before
    // serialization. The sender's independent RPC payment may still be retiring:
    // callback retirement is not last-Copy collection. Fixed output metadata is
    // paid by the unchanged physical Control envelope, not by those retired wrappers.
    // HeaderWorker still excludes another request through socket/output collection;
    // Task retirement is not an engine/SDK/whole-row settlement assertion.
    let mut reply = incoming.reply(BodyKind::Control, encoded.len(), None)?;
    reply.plain_mut().copy_from_slice(encoded.as_bytes());
    reply.seal()
}
async fn prepare(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<Prepared, Refused> {
    let (row, action, engine_bytes) = resolve(context, route, incoming)?;
    let history = row.sdp_history(route.session().identity())?;
    if history.status().action != Some((action.serial, action.version, action.input_tag)) {
        return Err(VoiceError::Unauthorized.into());
    }
    if history.status().engine.settled || history.status().engine.receipt.is_some() {
        return Ok(Prepared::History(Box::new(history)));
    }
    drop(history);
    queue(
        context,
        route,
        row,
        action,
        engine_bytes,
        incoming.hold_position().map_err(|error| match error {
            channel::Refused::Capacity => VoiceError::Capacity,
            _ => VoiceError::Unauthorized,
        })?,
    )?
    .complete()
    .await
    .map(Prepared::Completion)
}
fn resolve(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<(ResolvedRow, Action, u32), Refused> {
    if !incoming.belongs_to(route.session()) {
        return Err(VoiceError::Unauthorized.into());
    }
    let Request::Existing(request) = Request::decode(
        incoming.operation(),
        incoming.kind(),
        incoming.logical(),
        incoming.plain().map_err(|_| VoiceError::Malformed)?,
    )?
    else {
        return Err(VoiceError::Malformed.into());
    };
    let Kind::Admit {
        action,
        engine_bytes,
    } = request.kind
    else {
        return Err(VoiceError::Malformed.into());
    };
    let live = context.upgrade().ok_or(Refused::Closed)?;
    live.channels
        .require_cleanup_route(route)
        .map_err(|_| Refused::Closed)?;
    let row = live.ledger.resolve(
        route.selected(),
        super::ledger::Purpose::Sdp {
            parent: request.parent,
            purpose: request.purpose,
        },
        request.row,
    )?;
    Ok((row, action, engine_bytes))
}

#[allow(clippy::large_enum_variant)]
enum Encoded {
    Permit([u8; super::replies::PERMIT_BYTES]),
    History([u8; super::sdp_replies::HISTORY_BYTES]),
    Refused([u8; 4]),
}
impl Encoded {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Permit(b) => b,
            Self::History(b) => b,
            Self::Refused(b) => b,
        }
    }
    fn len(&self) -> usize {
        self.as_bytes().len()
    }
}
// Separate the callback-bearing canonical projection from array serialization. In
// particular, the220-byte encoded history does not coexist with Root history queries.
// This is fixed physical Control storage, not another native Phase contribution.
#[allow(clippy::large_enum_variant)]
enum Projection {
    Permit([u8; super::replies::PERMIT_BYTES]),
    History(Box<SdpHistory>),
    Refused(Refused),
}
fn project(route: &Route, prepared: Result<Prepared, Refused>) -> Projection {
    let result = match prepared {
        Ok(Prepared::History(history)) => Ok(Projection::History(history)),
        Ok(Prepared::Completion(output)) => {
            if let Ok(SdpEngineRead::Permit(serial)) = &output.result {
                super::sdp_replies::admitted(
                    output.target.descriptor.row,
                    output.target.descriptor.action,
                    output.target.action.engine_bytes(),
                    *serial,
                )
                .map(Projection::Permit)
                .map_err(Into::into)
            } else {
                // Callback is retired and UI was never stored in this output. Recover
                // canonical history outside all guards, including after stale refusal.
                match output.target.row().sdp_history(route.session().identity()) {
                    Ok(history)
                        if history.status().engine.settled
                            || history.status().engine.receipt.is_some() =>
                    {
                        Ok(Projection::History(Box::new(history)))
                    }
                    Ok(_) => Err(output
                        .result
                        .as_ref()
                        .err()
                        .copied()
                        .unwrap_or(VoiceError::Capacity.into())),
                    Err(error) => Err(error.into()),
                }
            }
        }
        Err(error) => Err(error),
    };
    result.unwrap_or_else(Projection::Refused)
}
fn serialize(projected: Projection) -> Encoded {
    match projected {
        Projection::Permit(bytes) => Encoded::Permit(bytes),
        Projection::History(history) => match super::sdp_replies::history(&history) {
            Ok(bytes) => Encoded::History(bytes),
            Err(error) => Encoded::Refused(super::sdp_replies::refused(error.into())),
        },
        Projection::Refused(error) => Encoded::Refused(super::sdp_replies::refused(error)),
    }
}

fn task_layout() -> usize {
    fn observer<F: std::future::Future>(_: fn(Waiter) -> F) -> usize {
        std::mem::size_of::<F>()
    }
    let shared = std::mem::size_of::<(usize, usize, Custody)>()
        + std::mem::size_of::<Publisher>()
        + std::mem::size_of::<Target>()
        + std::mem::size_of::<Completion>()
        + std::mem::size_of::<Box<Completion>>()
        + 512; // pinned Tokio1.53.3 oneshot payload, state and wakers; no spawned task
    let construction = std::mem::size_of::<Construction>()
        + std::mem::size_of::<Target>()
        + std::mem::size_of::<Publisher>()
        + std::mem::size_of::<Waiter>()
        + std::mem::size_of::<catcoms_app::ServerActor>()
        + 128;
    let callback = std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<crate::UiSessionGate<'static>>()
        + std::mem::size_of::<super::transition::Admission<'static>>()
        + std::mem::size_of::<std::sync::MutexGuard<'static, crate::voice_authority::Coordinator>>(
        )
        + 5 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + std::mem::size_of::<tokio::sync::OwnedMutexGuard<()>>()
        + std::mem::size_of::<Result<SdpEngineRead, Refused>>()
        + std::mem::size_of::<Completion>()
        + 128;
    let observe = observer(Waiter::complete) + std::mem::size_of::<Completion>() + 64;
    shared + construction.max(callback).max(observe)
}

#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    fn physical<F: std::future::Future>(
        _: fn(&'static Context, &'static Route, Incoming) -> F,
    ) -> usize {
        std::mem::size_of::<F>() + std::mem::size_of::<F::Output>()
    }
    let lookup = std::mem::size_of::<Request<'static>>()
        + std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<Result<(ResolvedRow, Action, u32), Refused>>()
        + 96;
    let history = std::mem::size_of::<ResolvedRow>()
        + std::mem::size_of::<Action>()
        + super::ledger::sdp_history_layout_bytes()
        + 96;
    let project = std::mem::size_of::<Result<Prepared, Refused>>()
        + std::mem::size_of::<Projection>()
        + super::ledger::sdp_history_layout_bytes()
        + 128;
    // serialization_layout_bytes already includes the220-byte array moved into
    // Encoded::History. Count Projection plus enum overhead, not that same array
    // twice. Canonical queries above return BEFORE any Encoded value is constructed.
    let serialize = std::mem::size_of::<SdpHistory>()
        + std::mem::size_of::<Projection>()
        + (std::mem::size_of::<Encoded>() - super::sdp_replies::HISTORY_BYTES)
        + super::sdp_replies::serialization_layout_bytes()
        + 96;
    let before_payment =
        std::mem::size_of::<Construction>() + super::ledger::sdp_input_prepayment_layout_bytes();
    let bytes = physical(admit)
        + lookup
            .max(history)
            .max(project)
            .max(serialize)
            .max(before_payment);
    eprintln!(
        "native SDP engine physical {bytes}, Task {}/{TASK_BYTES}; future/output {}, lookup {lookup}, history {history}, project {project}, serialize {serialize}, prepay {before_payment}",
        task_layout(), physical(admit)
    );
    bytes
}

/// Actual blocked-TCP native custody fixture. It admits a crypto permit but executes
/// no browser SDK, AEAD or RTC; explicit settlement below represents the fixture's
/// actual end of its admitted work, never a timeout/lost-reply inference.
#[cfg(test)]
pub(super) async fn exercise(
    state: &crate::AppState,
    public: &super::bootstrap::SetupOutput,
    route: &Route,
    parent_row: &ResolvedRow,
    pc_frame: &[u8],
    child: super::ledger::RowId,
    action: Action,
) {
    use super::service::tests::{opened_frame, opened_reply};
    use catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowPurpose;
    let context = Context::new(state);
    let (actor, _) = factory::native::observation_actor(&context, route).unwrap();
    let row = || {
        state
            .voice_handoff
            .resolve(
                route.selected(),
                super::ledger::Purpose::Sdp {
                    parent: parent_row.id(),
                    purpose: NativeRowPurpose::Seal,
                },
                child,
            )
            .unwrap()
    };
    let owners = row()
        .sdp_input_owners(route.session().identity(), action)
        .unwrap();
    let task = owners.phase.task_copy(TASK_BYTES).unwrap();
    let target = row()
        .sdp_input_target(route.session().identity(), action, &owners, &task)
        .unwrap();
    let original = target.action.clone();
    let funding = owners.funding.clone();
    drop((target, task));
    let bytes = u32::try_from(original.engine_bytes()).unwrap();
    let mut body = vec![8];
    body.extend_from_slice(&parent_row.id().logical());
    body.push(0);
    body.extend_from_slice(&action.serial.to_be_bytes());
    body.extend_from_slice(&action.version.to_be_bytes());
    body.extend_from_slice(&action.input_tag);
    body.extend_from_slice(&bytes.to_be_bytes());
    let header = || {
        (
            route.session().fixture_counts().1 + 1,
            18,
            0,
            child.logical(),
        )
    };
    let capacity = super::sdp_replies::refused(VoiceError::Capacity.into());

    // Partially prepared native Task/Actor admission uses the original16-Copy row
    // counter; exhaustion rolls back before an Actor command or engine exists.
    let mut pressure = Vec::new();
    for _ in 0..15 {
        let mut copy = owners.phase.copy(1).unwrap();
        copy.bind_native_history(funding.clone()).unwrap();
        pressure.push(copy);
    }
    let h = header();
    let reply = admit(&context, route, opened_frame(public, route, h, &body, None))
        .await
        .unwrap();
    assert_eq!(opened_reply(public, h, &body, &reply), capacity);
    drop(reply);
    assert_eq!(funding.in_flight(), 15);
    drop(pressure);

    // Park the ORIGINAL Actor and fill its actual finite mailbox. Synchronous native
    // refusal must keep sender/payment valid even if its receiver immediately collects.
    let parked = super::integration::pc::park(&actor).await;
    let mut queries = Vec::new();
    loop {
        match actor.try_queue_voice_v2_context(route.selected().binding().scope.room) {
            Ok(query) => queries.push(query),
            Err(VoiceEndpointError::Protocol(VoiceError::Capacity)) => break,
            Err(error) => panic!("unexpected mailbox {error:?}"),
        }
    }
    let h = header();
    let reply = admit(&context, route, opened_frame(public, route, h, &body, None))
        .await
        .unwrap();
    assert_eq!(opened_reply(public, h, &body, &reply), capacity);
    drop((reply, queries, parked));
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);

    // Lock wins AFTER positive queue admission. No cached generation/store flag can
    // authorize the first engine, and queued work itself must not retain UI.
    let parked = super::integration::pc::park(&actor).await;
    let h = header();
    let incoming = opened_frame(public, route, h, &body, None);
    let waiter = queue(
        &context,
        route,
        row(),
        action,
        bytes,
        incoming.hold_position().unwrap(),
    )
    .unwrap();
    assert!(waiter.work.is_some());
    let commit = state.ui_session_commit.clone().lock_owned().await;
    drop(parked);
    let output = waiter.complete().await.unwrap();
    assert!(matches!(
        output.result,
        Err(Refused::Protocol(VoiceError::Capacity))
    ));
    drop((output, incoming, commit));
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);

    // Lose an admitted request's observer. Only the SAME original Action can recover
    // its permit; loss is not unstarted work and cannot refund actual engine credit.
    let parked = super::integration::pc::park(&actor).await;
    let h = header();
    let incoming = opened_frame(public, route, h, &body, None);
    let waiter = queue(
        &context,
        route,
        row(),
        action,
        bytes,
        incoming.hold_position().unwrap(),
    )
    .unwrap();
    assert!(waiter.work.is_some());
    drop((waiter, incoming));
    assert_eq!(funding.in_flight(), 2);
    assert!(matches!(
        route.session().header(),
        Err(channel::Refused::Capacity)
    ));
    drop(parked);
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    route.session().fixture_wait_physical_idle().await;
    // The physical hold retires before the last Task payment; wait on a same-Actor
    // FIFO after callback return and check the actual counter, not physical-idle alone.
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);
    assert!(
        state.ui_session_commit.try_lock().is_ok(),
        "engine never stores UI in output"
    );

    let expected =
        super::sdp_replies::admitted(child, action, bytes as usize, original.serial()).unwrap();
    let clock = crate::voice_runtime(state).clock();
    for _ in 0..2 {
        let mut accepted = false;
        for _ in 0..32 {
            let h = header();
            let reply = admit(&context, route, opened_frame(public, route, h, &body, None))
                .await
                .unwrap();
            let result = opened_reply(public, h, &body, &reply);
            if result == capacity {
                drop(reply);
                clock.sleep(std::time::Duration::from_millis(10)).await;
                continue;
            }
            assert_eq!(result, expected);
            assert!(
                state.ui_session_commit.try_lock().is_ok(),
                "held permit metadata must not hold UI"
            );
            assert!(matches!(
                route.session().header(),
                Err(channel::Refused::Capacity)
            ));
            // InvocationRetirement wakes before the sender's last Work/Copy Drop.
            // A SAME-Actor FIFO barrier proves that turn actually returned; the held
            // physical reply deliberately remains uncollected through this observation.
            actor
                .voice_v2_context(route.selected().binding().scope.room)
                .await
                .unwrap();
            assert_eq!(
                funding.in_flight(),
                0,
                "fixed Control, not Task, pays serialized output"
            );
            drop(reply);
            accepted = true;
            break;
        }
        assert!(accepted, "same original permit within fixed attempts");
    }
    original.retire_if_unstarted();
    assert!(
        !original.handoff_status().engine.settled,
        "lost permit did admit one original engine"
    );

    // A pending retry still needs current native PC admission, even with its existing
    // permit. Close wins after queueing; it must not revive a replacement/closed PC.
    let parked = super::integration::pc::park(&actor).await;
    let h = header();
    let incoming = opened_frame(public, route, h, &body, None);
    let waiter = queue(
        &context,
        route,
        row(),
        action,
        bytes,
        incoming.hold_position().unwrap(),
    )
    .unwrap();
    let pc = super::requests::pc::Request::decode(parent_row.id().logical(), pc_frame).unwrap();
    parent_row
        .close_pc(route.session().identity(), &pc)
        .unwrap();
    drop(parked);
    let output = waiter.complete().await.unwrap();
    assert!(matches!(
        output.result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    drop((output, incoming));
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);
    // No SDK ran and the actual native engine flight has joined. Settle through the
    // authenticated original23 control AFTER PC closure, with no terminal to ACK.
    // A local try-lock Capacity may retry the same metadata; no fresh engine is lent.
    let settle = &body[..66];
    let clock = crate::voice_runtime(state).clock();
    let cleanup = async {
        loop {
            let h = (
                route.session().fixture_counts().1 + 1,
                23,
                0,
                child.logical(),
            );
            let reply = super::sdp_http::apply(
                &context,
                route,
                opened_frame(public, route, h, settle, None),
            )
            .await
            .unwrap();
            let history = opened_reply(public, h, settle, &reply);
            drop(reply);
            if history != capacity {
                break history;
            }
            clock.sleep(std::time::Duration::from_millis(10)).await;
        }
    };
    let history = tokio::select! {
        biased;
        _ = clock.sleep(std::time::Duration::from_secs(15)) => panic!("no-result native settlement exceeded harness bound"),
        history = cleanup => history,
    };
    assert_eq!(&history[..2], &[8, 0]);
    assert_eq!(history[86], 1);
    assert_eq!(
        history[87], 0,
        "settlement never invents a terminal receipt"
    );
    assert_eq!(
        history[186], 0,
        "no-result settlement needs no invented ACK"
    );
    assert!(original.input().is_err());
    let h = header();
    let reply = admit(&context, route, opened_frame(public, route, h, &body, None))
        .await
        .unwrap();
    let history = opened_reply(public, h, &body, &reply);
    assert_eq!(&history[..2], &[8, 0]);
    assert_eq!(history.len(), 220);
    assert_eq!(history[86], 1);
    assert_eq!(
        funding.in_flight(),
        0,
        "closed history requires no new Task/Actor Copy"
    );
    drop(reply);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdp_engine_concrete_publisher_and_observer_fit_original_task_payment() {
        let bytes = task_layout();
        eprintln!("native SDP engine Task {bytes}/{TASK_BYTES}");
        assert!(bytes <= TASK_BYTES);
    }
}
