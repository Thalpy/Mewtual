//! Private original SDP Input producer. No factory/registration, replacement input or
//! current-authority reply crosses an await. Authenticated service dispatch uses this
//! producer; App V2 call selection and real media acceptance remain separate.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, PhysicalHold};
use super::context::Context;
use super::factory::{self, Refused};
use super::ledger::{ResolvedRow, SdpHistory, SdpInputTarget as Target, SdpSubmissionOwners};
use super::requests::sdp_action::{self, Kind, Request};
use super::requests::Action;
use super::sdp_input_codec::View;
use super::sdp_publication::TASK_BYTES;
use catcoms_app::{
    VoiceEndpointError, VoiceSdpInputPublisher, VoiceSdpInputWork, VOICE_SDP_INPUT_RPC_BYTES,
};
use catcoms_sync::voice_v2::chain::sdp_handoff::{Copy, TaskCopy};
use catcoms_sync::voice_v2::crypto_actions::{SdpAdmittedInput, SdpInputRead};
use catcoms_sync::voice_v2::sdp::SdpInputAdmission;
use catcoms_sync::voice_v2::VoiceError;
use std::sync::{Arc, OnceLock};
use tokio::sync::oneshot;

/// Independently paid before allocating any concrete publisher/channel/future. OnceLock
/// installs the ACTUAL UI guard exactly once after successful Root publication. Observer
/// references can exist beforehand; they cannot install, release or inspect permission.
struct Custody {
    commit: OnceLock<tokio::sync::OwnedMutexGuard<()>>,
    _physical: PhysicalHold,
    // LAST. TaskCopy cannot adopt/keepalive, preventing a response->payment->response cycle.
    _task: TaskCopy,
}
struct Publisher {
    context: Context,
    route: Route,
    target: Option<Box<Target>>,
    reply: Option<oneshot::Sender<Box<Completion>>>,
    response: Option<Copy>,
    // LAST. Box/arguments/synchronous send stay paid even if its receiver collects early.
    custody: Arc<Custody>,
}
struct Completion {
    result: Result<SdpInputRead, Refused>,
    target: Box<Target>,
    response: Option<Copy>,
    _work: Option<VoiceSdpInputWork>,
    // LAST: actual input/row/Actor work dies before UI/physical/task custody.
    _custody: Arc<Custody>,
}
impl std::fmt::Debug for Completion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SdpInputCompletion { .. }")
    }
}
/// Data must not retain Target/ResolvedRow: that would pin the process Ledger and all
/// unrelated histories after Host/AppState destruction. Only actual input, RPC and this
/// publication's own independent custody survive with encrypted response bytes.
struct DataKeepalive {
    _input: SdpAdmittedInput,
    _work: Option<VoiceSdpInputWork>,
    _custody: Arc<Custody>,
}
impl std::fmt::Debug for DataKeepalive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SdpInputDataKeepalive { .. }")
    }
}
#[allow(clippy::large_enum_variant)]
enum Prepared {
    History(Box<SdpHistory>),
    Completion(Box<Completion>),
}
struct Waiter {
    reply: oneshot::Receiver<Box<Completion>>,
    work: Option<VoiceSdpInputWork>,
    // Native payment ALSO covers a retained or cancelled receiver/future.
    _custody: Arc<Custody>,
}
impl Waiter {
    async fn complete(mut self) -> Result<Box<Completion>, Refused> {
        let result = (&mut self.reply).await.map_err(|_| Refused::Stopped)?;
        if let Some(work) = &self.work {
            // This is callback retirement, not collection. Data may still retain its
            // admitted UI guard. History has installed NONE and may recover after here.
            work.wait_retired().await;
        }
        Ok(result)
    }
}
/// One ordered intake guard exists before any fallible payment/target preparation.
/// Original target/channel metadata always drops before the last independent TaskCopy.
struct Construction {
    target: Option<Box<Target>>,
    owners: Option<SdpSubmissionOwners>,
    rpc: Option<Copy>,
    response: Option<Copy>,
    custody: Option<Arc<Custody>>,
    physical: Option<PhysicalHold>,
    task: Option<TaskCopy>,
}
fn queue(
    context: &Context,
    route: &Route,
    row: ResolvedRow,
    action: Action,
    physical: PhysicalHold,
) -> Result<Waiter, Refused> {
    let mut construction = Construction {
        target: None,
        owners: None,
        rpc: None,
        response: None,
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
        .expect("prepaid native graph")
        .bind_native_history(owners.funding.clone())?;
    // This full descriptor/Target allocation is covered by TaskCopy, not the tight
    // permanent row fee used for the small SubmissionOwners prepayment snapshot.
    construction.target = Some(Box::new(row.sdp_input_target(
        route.session().identity(),
        action,
        owners,
        construction.task.as_ref().expect("original task"),
    )?));
    construction.rpc = Some(owners.phase.copy(VOICE_SDP_INPUT_RPC_BYTES)?);
    construction
        .rpc
        .as_mut()
        .expect("original actor payment")
        .bind_native_history(owners.funding.clone())?;
    construction.response = Some(owners.phase.copy(sdp_action::INPUT_CIPHER_BYTES)?);
    construction
        .response
        .as_mut()
        .expect("original response payment")
        .bind_native_history(owners.funding.clone())?;
    construction.custody = Some(Arc::new(Custody {
        commit: OnceLock::new(),
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
        .expect("original task custody")
        .clone();
    let (reply, receiver) = oneshot::channel();
    let publisher = Box::new(Publisher {
        context: context.clone(),
        route: route.clone(),
        target: Some(target),
        reply: Some(reply),
        response: construction.response.take(),
        custody: custody.clone(),
    });
    let work = actor
        .try_queue_voice_sdp_input(
            grant,
            intent,
            original,
            construction.rpc.take().expect("original RPC payment"),
            publisher,
        )
        .ok();
    // Full/stopped is synchronously delivered by the SAME consuming hook. In either
    // case retain the original receiver; its actual output is never silently discarded.
    Ok(Waiter {
        reply: receiver,
        work,
        _custody: custody,
    })
}
impl Publisher {
    fn admit(&self, step: SdpInputAdmission<'_>) -> Result<SdpInputRead, Refused> {
        let live = self.context.upgrade().ok_or(Refused::Closed)?;
        let commit = live
            .commit
            .clone()
            .try_lock_owned()
            .map_err(|_| VoiceError::Capacity)?;
        // Generation/selected-owner checks alone do not prove a mounted unlocked Store.
        // Repeat the actual resumable/store try-locks AFTER the queued Actor wait and
        // inside this same UI publication fence. No cached preflight bool survives it.
        factory::require_unlocked_now(&live, self.route.selected())?;
        let result = {
            let authority = match live.authority.try_lock() {
                Ok(authority) => authority,
                Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity.into()),
            };
            let admission =
                super::transition::Admission::new(&live, &authority, &self.route, &commit);
            self.route.try_with_open(|| {
                self.target
                    .as_ref()
                    .expect("original target")
                    .admit(&admission, || step.commit())
            })
        };
        // Every short native and Root guard has returned before any result/loan Drop,
        // allocation, attachment or serialization. Only actual Data retains the UI gate.
        if matches!(result, Ok(SdpInputRead::Data(_))) {
            assert!(
                self.custody.commit.set(commit).is_ok(),
                "one original Input publication"
            );
        } else {
            drop(commit);
        }
        result.map_err(Into::into)
    }
    fn complete(
        mut self: Box<Self>,
        result: Result<SdpInputRead, Refused>,
        work: Option<VoiceSdpInputWork>,
    ) {
        let completion = Box::new(Completion {
            result,
            target: self.target.take().expect("original target"),
            response: self.response.take(),
            _work: work,
            _custody: self.custody.clone(),
        });
        let _ = self
            .reply
            .take()
            .expect("one original reply")
            .send(completion);
        // self still owns independent custody through send AND this callback's return.
    }
}
impl VoiceSdpInputPublisher for Publisher {
    fn publish(self: Box<Self>, step: SdpInputAdmission<'_>, work: VoiceSdpInputWork) {
        let result = self.admit(step);
        self.complete(result, Some(work));
    }
    fn refused(self: Box<Self>, error: VoiceEndpointError) {
        self.complete(Err(error.into()), None);
    }
}

/// Authenticated staged handler only; no production listener dispatch is enabled here.
pub(super) async fn pull(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, channel::Refused> {
    let physical = incoming.hold_position()?;
    let result = prepare(context, route, &incoming, physical).await;
    match result {
        Ok(Prepared::Completion(completion))
            if matches!(completion.result, Ok(SdpInputRead::Data(_))) =>
        {
            data(incoming, completion)
        }
        other => control(incoming, route, other),
    }
}
async fn prepare(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
    physical: PhysicalHold,
) -> Result<Prepared, Refused> {
    let (row, action) = resolve_input(context, route, incoming)?;
    // No native/Root guard crosses this original observation. A known terminal/settled
    // read should use History directly instead of trying to create new input capacity.
    let history = row.sdp_history(route.session().identity())?;
    if history.status().action != Some((action.serial, action.version, action.input_tag)) {
        return Err(VoiceError::Unauthorized.into());
    }
    // Terminal history cannot require a fresh Copy from closed/collected storage. It is
    // returned as Control by the separate path below, without enqueuing a native hook.
    if history.status().engine.settled || history.status().engine.receipt.is_some() {
        return Ok(Prepared::History(Box::new(history)));
    }
    drop(history);
    queue(context, route, row, action, physical)?
        .complete()
        .await
        .map(Prepared::Completion)
}
/// The complete borrowed grammar/context lookup occupies this synchronous frame only.
/// Neither its large union nor mounted context coexists with canonical Root history or
/// remains in the physical async frame while a paid native Actor publisher is pending.
fn resolve_input(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<(ResolvedRow, Action), Refused> {
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
    let Kind::Input(action) = request.kind else {
        return Err(VoiceError::Malformed.into());
    };
    let row = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        live.channels
            .require_cleanup_route(route)
            .map_err(|_| Refused::Closed)?;
        live.ledger.resolve(
            route.selected(),
            super::ledger::Purpose::Sdp {
                parent: request.parent,
                purpose: request.purpose,
            },
            request.row,
        )?
    };
    Ok((row, action))
}

fn data(
    incoming: Incoming,
    mut completion: Box<Completion>,
) -> Result<CompletedReply, channel::Refused> {
    let Ok(SdpInputRead::Data(loan)) = &completion.result else {
        return Err(channel::Refused::Binding);
    };
    let view = View::admitted(&completion.target, loan).map_err(|_| channel::Refused::Binding)?;
    let length = view.plain_bytes();
    let response = completion
        .response
        .take()
        .ok_or(channel::Refused::Binding)?;
    let mut buffer = response
        .adopt(vec![0; length + 16])
        .map_err(|_| channel::Refused::Capacity)?;
    view.encode(&mut buffer.bytes_mut()[..length])
        .map_err(|_| channel::Refused::Binding)?;
    let Completion {
        result,
        target,
        response,
        _work,
        _custody,
    } = *completion;
    let Ok(SdpInputRead::Data(input)) = result else {
        unreachable!("checked original Data");
    };
    // Pack ordered custody BEFORE any cleanup which can unwind. In particular final
    // Target/Ledger destruction cannot drop a last UI/Task owner ahead of input/Work.
    let keepalive = DataKeepalive {
        _input: input,
        _work,
        _custody,
    };
    debug_assert!(response.is_none());
    drop(target); // final broad Ledger/selected handles retire before output retention
                  // This owner contains neither Target/Ledger nor this Copy/Buffer: no custody cycle
                  // and no unrelated row/registry can be pinned by a delayed or abandoned HTTP output.
    buffer
        .attach_keepalive(Arc::new(keepalive))
        .map_err(|_| channel::Refused::Binding)?;
    incoming.reply_sdp(buffer)?.seal()
}
fn control(
    incoming: Incoming,
    route: &Route,
    completion: Result<Prepared, Refused>,
) -> Result<CompletedReply, channel::Refused> {
    let result = match completion {
        Ok(Prepared::History(history)) => Ok(history),
        Ok(Prepared::Completion(completion)) => {
            // No History/refusal branch installed a UI guard. Callback retirement has
            // already completed; canonical recovery now runs outside every native guard.
            let original_error = completion.result.as_ref().err().copied();
            let history = completion
                .target
                .row()
                .sdp_history(route.session().identity());
            match history {
                Ok(history)
                    if history.status().engine.settled
                        || history.status().engine.receipt.is_some() =>
                {
                    Ok(Box::new(history))
                }
                Ok(_) => Err(original_error.unwrap_or(VoiceError::Capacity.into())),
                Err(error) => Err(error.into()),
            }
        }
        Err(error) => Err(error),
    };
    let mut reply = incoming.reply(
        BodyKind::Control,
        if result.is_ok() {
            super::sdp_replies::HISTORY_BYTES
        } else {
            4
        },
        None,
    )?;
    match result {
        Ok(snapshot) => reply.plain_mut().copy_from_slice(
            &super::sdp_replies::history(&snapshot).map_err(|_| channel::Refused::Binding)?,
        ),
        Err(error) => reply
            .plain_mut()
            .copy_from_slice(&super::sdp_replies::refused(error)),
    }
    reply.seal()
}

/// Whole independently paid concrete graph, including actual boxed publisher, oneshot
/// payload storage, observer/future/output and synchronous constructor/admission frames.
/// Original Row/Action payload allocations and Root/App's positive4KiB RPC have their own
/// retained payments. None is substituted for these concrete native wrappers.
fn task_layout() -> usize {
    #[repr(C)]
    struct ArcAllocation<T> {
        strong: usize,
        weak: usize,
        value: T,
    }
    fn waiter_future<F: std::future::Future>(_: fn(Waiter) -> F) -> usize {
        std::mem::size_of::<F>()
    }
    // Pinned Tokio1.53.3 oneshot: payload storage plus state, counters and both wakers.
    // Conservatively reserve512B beyond the actual payload (not another task/spawn).
    let shared = std::mem::size_of::<ArcAllocation<Custody>>()
        + std::mem::size_of::<Publisher>()
        + std::mem::size_of::<Target>() // one separately prepaid affine Target Box
        + std::mem::size_of::<Completion>() // native-paid completed payload Box
        + std::mem::size_of::<Box<Completion>>() // oneshot payload storage
        + std::mem::size_of::<ArcAllocation<DataKeepalive>>()
        + 512;
    let construction = std::mem::size_of::<Construction>()
        + std::mem::size_of::<Target>()
        + std::mem::size_of::<Publisher>()
        + std::mem::size_of::<Waiter>()
        + std::mem::size_of::<catcoms_app::ServerActor>()
        + 128; // original Arc/oneshot/permit argument handles and return discriminants
    let callback = std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<crate::UiSessionGate<'static>>()
        + std::mem::size_of::<super::transition::Admission<'static>>()
        + std::mem::size_of::<std::sync::MutexGuard<'static, crate::voice_authority::Coordinator>>(
        )
        + 5 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + std::mem::size_of::<tokio::sync::OwnedMutexGuard<()>>()
        + std::mem::size_of::<Result<SdpInputRead, Refused>>()
        + std::mem::size_of::<Completion>()
        + 128;
    let observer = waiter_future(Waiter::complete) + std::mem::size_of::<Completion>() + 64;
    let serialize = std::mem::size_of::<Completion>()
        + std::mem::size_of::<View<'static>>()
        + std::mem::size_of::<Target>()
        + std::mem::size_of::<DataKeepalive>()
        + std::mem::size_of::<catcoms_sync::voice_v2::chain::sdp_handoff::Buffer>()
        + std::mem::size_of::<Copy>()
        + 192;
    shared + construction.max(callback).max(observer).max(serialize)
}

/// Inline caller graph still belongs to the unchanged physical Control envelope. The
/// independently paid concrete publisher/oneshot/future/boxed completion is inventoried
/// above; only caller arguments surround queueing. Known-history needs no new TaskCopy.
#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    fn measured<F: std::future::Future>(
        _: fn(&'static Context, &'static Route, Incoming) -> F,
    ) -> usize {
        std::mem::size_of::<F>() + std::mem::size_of::<F::Output>()
    }
    let lookup = std::mem::size_of::<Request<'static>>()
        + std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<Result<(ResolvedRow, Action), Refused>>()
        + 96;
    let inspect = std::mem::size_of::<ResolvedRow>()
        + std::mem::size_of::<Action>()
        + 96
        + super::ledger::sdp_history_layout_bytes();
    let control = std::mem::size_of::<Result<Prepared, Refused>>()
        + 96
        + super::ledger::sdp_history_layout_bytes();
    // Canonical history returns BEFORE its Box is allocated. That separately paid
    // physical Box exists only during serialization, never beside the history helper.
    let serialize =
        std::mem::size_of::<SdpHistory>() + 96 + super::sdp_replies::serialization_layout_bytes();
    let prepayment =
        std::mem::size_of::<Construction>() + super::ledger::sdp_input_prepayment_layout_bytes();
    let nested = lookup
        .max(inspect)
        .max(control)
        .max(serialize)
        .max(prepayment)
        .max(128);
    let bytes = measured(pull) + nested;
    eprintln!("SDP Input physical adapter {bytes}; future/output {}; lookup {lookup}, history {inspect}, control {control}, serialize {serialize}; native task {}", measured(pull), task_layout());
    bytes
}

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
    use super::service::tests::{opened_frame, opened_reply, opened_reply_kind};
    use catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowPurpose;
    let context = Context::new(state);
    let parent = parent_row.id();
    let (actor, _) = factory::native::observation_actor(&context, route).unwrap();
    let row = || {
        state
            .voice_handoff
            .resolve(
                route.selected(),
                super::ledger::Purpose::Sdp {
                    parent,
                    purpose: NativeRowPurpose::Seal,
                },
                child,
            )
            .unwrap()
    };
    let original = row()
        .sdp_input_owners(route.session().identity(), action)
        .unwrap();
    let funding = original.funding.clone();
    let mut body = vec![8];
    body.extend_from_slice(&parent.logical());
    body.push(0);
    body.extend_from_slice(&action.serial.to_be_bytes());
    body.extend_from_slice(&action.version.to_be_bytes());
    body.extend_from_slice(&action.input_tag);
    let header = || {
        (
            route.session().fixture_counts().1 + 1,
            17,
            0,
            child.logical(),
        )
    };

    // Actual authenticated request -> consuming App hook -> scoped Root input -> exact
    // class2 response. It installs no engine or replacement action.
    let h = header();
    let incoming = opened_frame(public, route, h, &body, None);
    let reply = pull(&context, route, incoming).await.unwrap();
    let bytes = opened_reply_kind(public, h, &body, &reply, 2);
    assert_eq!(&bytes[..2], &[8, 3]);
    assert_eq!(&bytes[2..18], &child.logical());
    assert_eq!(&bytes[18..34], &parent.logical());
    assert_eq!(&bytes[34..42], &action.serial.to_be_bytes());
    assert_eq!(&bytes[271..273], &[0, 0]);
    let aad = u16::from_be_bytes(bytes[273..275].try_into().unwrap()) as usize;
    assert_eq!(&bytes[277 + aad..], b"v=0");
    assert_eq!(
        funding.in_flight(),
        3,
        "native task, Actor work and response remain paid"
    );
    assert!(
        state.ui_session_commit.try_lock().is_err(),
        "actual admitted response retains UI commit"
    );
    assert!(matches!(
        route.session().header(),
        Err(channel::Refused::Capacity)
    ));
    drop(reply);
    assert!(state.ui_session_commit.try_lock().is_ok());
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);

    // The fixed native counter's own16-Copy limit is independent of Phase bytes. A
    // partial three-owner construction must roll back before any Actor command exists.
    let mut pressure = Vec::new();
    for _ in 0..15 {
        let mut copy = original.phase.copy(1).unwrap();
        copy.bind_native_history(funding.clone()).unwrap();
        pressure.push(copy);
    }
    let h = header();
    let reply = pull(&context, route, opened_frame(public, route, h, &body, None))
        .await
        .unwrap();
    assert_eq!(
        opened_reply(public, h, &body, &reply),
        super::sdp_replies::refused(VoiceError::Capacity.into())
    );
    drop(reply);
    assert_eq!(
        funding.in_flight(),
        15,
        "partially constructed task/RPC rolled back"
    );
    drop(pressure);
    assert_eq!(funding.in_flight(), 0);

    // The ORIGINAL Actor is parked, then its actual finite mailbox is filled using
    // existing read-only queries. Full must invoke THIS publisher's synchronous refusal
    // and retire all three prepayments, never just queue an unobserved fake success.
    let parked = super::integration::pc::park(&actor).await;
    let mut queries = Vec::new();
    loop {
        match actor.try_queue_voice_v2_context(route.selected().binding().scope.room) {
            Ok(query) => queries.push(query),
            Err(VoiceEndpointError::Protocol(VoiceError::Capacity)) => break,
            Err(error) => panic!("unexpected actual mailbox refusal {error:?}"),
        }
    }
    let h = header();
    let reply = pull(&context, route, opened_frame(public, route, h, &body, None))
        .await
        .unwrap();
    assert_eq!(
        opened_reply(public, h, &body, &reply),
        super::sdp_replies::refused(VoiceError::Capacity.into())
    );
    drop(reply);
    assert_eq!(funding.in_flight(), 0);
    drop(queries);
    drop(parked);
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();

    // Native UI wins final admission after original Root time/preparation. No Data,
    // engine or retained UI owner can be manufactured from the earlier unlocked read.
    let held = state.ui_session_commit.clone().lock_owned().await;
    let h = header();
    let reply = pull(&context, route, opened_frame(public, route, h, &body, None))
        .await
        .unwrap();
    assert_eq!(
        opened_reply(public, h, &body, &reply),
        super::sdp_replies::refused(VoiceError::Capacity.into())
    );
    drop(reply);
    drop(held);
    assert_eq!(funding.in_flight(), 0);

    // Remove the actual mounted Store AFTER queue admission without changing generation.
    // The final publisher must repeat the mount check, not rely on selection/preflight.
    let parked = super::integration::pc::park(&actor).await;
    let h = header();
    let incoming = opened_frame(public, route, h, &body, None);
    let waiter = queue(
        &context,
        route,
        row(),
        action,
        incoming.hold_position().unwrap(),
    )
    .unwrap();
    assert!(waiter.work.is_some());
    let stored = state
        .store
        .lock()
        .await
        .take()
        .expect("actual mounted fixture Store");
    drop(parked);
    let completed = waiter.complete().await.unwrap();
    assert!(matches!(completed.result, Err(Refused::Closed)));
    *state.store.lock().await = Some(stored);
    drop(completed);
    drop(incoming);
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);

    // Actual resumable-lock contention after enqueue is retryable Capacity. No wait or
    // cached mounted flag may carry original Clock observations through this boundary.
    let parked = super::integration::pc::park(&actor).await;
    let h = header();
    let incoming = opened_frame(public, route, h, &body, None);
    let waiter = queue(
        &context,
        route,
        row(),
        action,
        incoming.hold_position().unwrap(),
    )
    .unwrap();
    let held = state.session_resumable.lock().await;
    drop(parked);
    let completed = waiter.complete().await.unwrap();
    assert!(matches!(
        completed.result,
        Err(Refused::Protocol(VoiceError::Capacity))
    ));
    drop(held);
    drop(completed);
    drop(incoming);
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);

    // Lose the actual queued observer and physical caller. The admitted sender remains
    // paid and busy; synchronous failed delivery must destroy input before its own task.
    let parked = super::integration::pc::park(&actor).await;
    let h = header();
    let incoming = opened_frame(public, route, h, &body, None);
    let waiter = queue(
        &context,
        route,
        row(),
        action,
        incoming.hold_position().unwrap(),
    )
    .unwrap();
    assert!(waiter.work.is_some());
    drop(waiter);
    drop(incoming);
    assert_eq!(funding.in_flight(), 3);
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
    assert_eq!(funding.in_flight(), 0);
    assert!(state.ui_session_commit.try_lock().is_ok());

    // Actual PC close wins AFTER queueing, with no forged native-current boolean.
    // The old intent cannot regain input after that irreversible storage fence.
    let parked = super::integration::pc::park(&actor).await;
    let h = header();
    let incoming = opened_frame(public, route, h, &body, None);
    let waiter = queue(
        &context,
        route,
        row(),
        action,
        incoming.hold_position().unwrap(),
    )
    .unwrap();
    let pc = super::requests::pc::Request::decode(parent.logical(), pc_frame).unwrap();
    parent_row
        .close_pc(route.session().identity(), &pc)
        .unwrap();
    drop(parked);
    let completed = waiter.complete().await.unwrap();
    assert!(matches!(
        completed.result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    drop(completed);
    drop(incoming);
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdp_input_concrete_publisher_channel_and_observer_fit_original_task_payment() {
        let bytes = task_layout();
        eprintln!("actual native SDP Input task graph {bytes}/{TASK_BYTES}");
        assert!(
            bytes <= TASK_BYTES,
            "concrete native Input graph needs {bytes}/{TASK_BYTES}"
        );
    }
}
