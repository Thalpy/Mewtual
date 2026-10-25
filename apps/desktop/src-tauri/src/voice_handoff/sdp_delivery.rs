//! Private SDP delivery admission through the actual Actor. Native pays only this
//! metadata queue/publication graph; RTC execution and its actual drainage remain
//! independently owned by the original private browser. Production SDP dispatch is still disabled.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, PhysicalHold};
use super::context::Context;
use super::factory::{self, Refused};
use super::ledger::{ResolvedRow, SdpHistory, SdpInputTarget as Target, SdpSubmissionOwners};
use super::requests::sdp_action::{Kind, Request};
use super::requests::Action;
use super::sdp_publication::TASK_BYTES;
use catcoms_app::{
    VoiceEndpointError, VoiceSdpDeliveryPublisher, VoiceSdpDeliveryWork,
    VOICE_SDP_DELIVERY_RPC_BYTES,
};
use catcoms_sync::voice_v2::chain::sdp_handoff::{Copy, TaskCopy};
#[cfg(test)]
use catcoms_sync::voice_v2::crypto_actions::CryptoAction;
use catcoms_sync::voice_v2::sdp::{SdpDelivery, SdpDeliveryAdmission};
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
    delivery: Arc<SdpDelivery>,
    applied: bool,
    reply: Option<oneshot::Sender<Box<Completion>>>,
    // The test barrier owns NO Task/physical/Work reference. It parks inside the
    // actual borrowed send tail while this same Publisher Box remains alive.
    #[cfg(test)]
    after_send: Option<tests::AfterSend>,
    // LAST: an immediate receiver cannot refund this synchronous sender's return.
    custody: Arc<Custody>,
}
struct Completion {
    result: Result<(), Refused>,
    applied: bool,
    target: Box<Target>,
    _work: Option<VoiceSdpDeliveryWork>,
    _custody: Arc<Custody>,
}
impl std::fmt::Debug for Completion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SdpDeliveryCompletion { .. }")
    }
}
struct Waiter {
    reply: oneshot::Receiver<Box<Completion>>,
    work: Option<VoiceSdpDeliveryWork>,
    _custody: Arc<Custody>,
}
impl Waiter {
    async fn complete(mut self) -> Result<Box<Completion>, Refused> {
        let output = (&mut self.reply).await.map_err(|_| Refused::Stopped)?;
        if let Some(work) = &self.work {
            // Root is still returning from its original callback after a synchronous
            // send. History observes Clock only after that actual invocation retires.
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
    applied: bool,
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
    construction.rpc = Some(owners.phase.copy(VOICE_SDP_DELIVERY_RPC_BYTES)?);
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
    let delivery = target
        .action
        .recover_sdp_open_history()?
        .and_then(|history| history.delivery)
        .ok_or(VoiceError::Unauthorized)?;
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
        delivery: delivery.clone(),
        applied,
        reply: Some(reply),
        #[cfg(test)]
        after_send: None,
        custody: custody.clone(),
    });
    // The same consuming publisher synchronously sends any local refusal. Its
    // observer still owns original Task custody through collection of that output.
    let work = actor
        .try_queue_voice_sdp_delivery(
            grant,
            intent,
            original,
            delivery,
            applied,
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
    fn admit(&self, step: SdpDeliveryAdmission<'_>) -> Result<(), Refused> {
        let live = self.context.upgrade().ok_or(Refused::Closed)?;
        let target = self.target.as_ref().expect("original target");
        // Verify BEFORE any delivery effect: a same-intent Action supplied to App is
        // not necessarily this indexed native row's actual registered allocation.
        if !step.matches(&target.action, &self.delivery, self.applied) {
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
        // origin after this hook returns; retaining UI in output would hide a
        // callback/credit destructor beneath native admission. Only actual original
        // delivery history remains independently paid by its original Action.
        result.map_err(Into::into)
    }
    fn complete(&mut self, result: Result<(), Refused>, work: Option<VoiceSdpDeliveryWork>) {
        let output = Box::new(Completion {
            result,
            applied: self.applied,
            target: self.target.take().expect("original target"),
            _work: work,
            _custody: self.custody.clone(),
        });
        let _ = self.reply.take().expect("one original reply").send(output);
        #[cfg(test)]
        if let Some(barrier) = self.after_send.take() {
            barrier.park();
        }
        // The consuming trait callback retains its ACTUAL Publisher Box through
        // this borrowed helper's send and return, including an immediate receiver.
        // Its last custody pays failed send and that callback's eventual return.
    }
}
impl VoiceSdpDeliveryPublisher for Publisher {
    fn publish(mut self: Box<Self>, step: SdpDeliveryAdmission<'_>, work: VoiceSdpDeliveryWork) {
        let result = self.admit(step);
        self.complete(result, Some(work));
    }
    fn historical(mut self: Box<Self>, work: VoiceSdpDeliveryWork) {
        self.complete(Ok(()), Some(work));
    }
    fn refused(mut self: Box<Self>, error: VoiceEndpointError) {
        self.complete(Err(error.into()), None);
    }
}

enum Prepared {
    History(Box<SdpHistory>),
    Completion(Box<Completion>),
}

/// Staged authenticated handler only. The shipping Host continues to refuse opcode26
/// with the rest of the SDP family before allocation or request-counter debit.
pub(super) async fn apply(
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
    let (row, action, applied) = resolve(context, route, incoming)?;
    if applied && historical_applied(&row, route, action)? {
        return Ok(Prepared::History(Box::new(
            row.sdp_history(route.session().identity())?,
        )));
    }
    queue(
        context,
        route,
        row,
        action,
        applied,
        incoming.hold_position().map_err(|error| match error {
            channel::Refused::Capacity => VoiceError::Capacity,
            _ => VoiceError::Unauthorized,
        })?,
    )?
    .complete()
    .await
    .map(Prepared::Completion)
}
fn historical_applied(row: &ResolvedRow, route: &Route, action: Action) -> Result<bool, Refused> {
    let original = row.sdp_result_action(route.session().identity(), action)?;
    let result = original
        .recover_sdp_open_history()?
        .ok_or(VoiceError::Unauthorized)?;
    let delivery = result.delivery.ok_or(VoiceError::Unauthorized)?;
    Ok(delivery.was_applied())
}
fn resolve(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<(ResolvedRow, Action, bool), Refused> {
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
    let Kind::Delivery { action, applied } = request.kind else {
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
    if request.purpose != catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowPurpose::Open {
        return Err(VoiceError::Unauthorized.into());
    }
    Ok((row, action, applied))
}

#[allow(clippy::large_enum_variant)]
enum Encoded {
    History([u8; super::sdp_replies::HISTORY_BYTES]),
    Refused([u8; 4]),
}
impl Encoded {
    fn as_bytes(&self) -> &[u8] {
        match self {
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
    History(Box<SdpHistory>),
    Refused(Refused),
}
fn project(route: &Route, prepared: Result<Prepared, Refused>) -> Projection {
    let result = match prepared {
        Ok(Prepared::History(history)) => Ok(Projection::History(history)),
        Ok(Prepared::Completion(output)) => {
            // Actual callback retirement precedes this Clock-bearing canonical recovery.
            // An old Admitted observation cannot contradict a fresh admission refusal.
            let canonical = match output.result {
                Ok(()) => Ok(true),
                Err(error) if output.applied => {
                    historical_applied(output.target.row(), route, output.target.descriptor.action)
                        .and_then(|recorded| if recorded { Ok(true) } else { Err(error) })
                }
                Err(error) => Err(error), // Fresh false26 must never become old-history success.
            };
            match canonical {
                Ok(_) => output
                    .target
                    .row()
                    .sdp_history(route.session().identity())
                    .map(|history| Projection::History(Box::new(history)))
                    .map_err(Into::into),
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    };
    result.unwrap_or_else(Projection::Refused)
}
fn serialize(projected: Projection) -> Encoded {
    match projected {
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
        + super::ledger::sdp_history_layout_bytes()
        + 128;
    let callback = std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<crate::UiSessionGate<'static>>()
        + std::mem::size_of::<super::transition::Admission<'static>>()
        + std::mem::size_of::<std::sync::MutexGuard<'static, crate::voice_authority::Coordinator>>(
        )
        + 5 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + std::mem::size_of::<tokio::sync::OwnedMutexGuard<()>>()
        + std::mem::size_of::<Result<(), Refused>>()
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
        + std::mem::size_of::<Result<(ResolvedRow, Action, bool), Refused>>()
        + 96;
    let history = std::mem::size_of::<ResolvedRow>()
        + std::mem::size_of::<Action>()
        + super::ledger::sdp_history_layout_bytes()
        + std::mem::size_of::<Arc<CryptoAction>>()
        + std::mem::size_of::<Arc<SdpDelivery>>()
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
    let bytes = physical(apply)
        + lookup
            .max(history)
            .max(project)
            .max(serialize)
            .max(before_payment);
    eprintln!(
        "native SDP delivery physical {bytes}, Task {}/{TASK_BYTES}; future/output {}, lookup {lookup}, history {history}, project {project}, serialize {serialize}, prepay {before_payment}",
        task_layout(), physical(apply)
    );
    bytes
}

#[cfg(test)]
pub(super) mod tests;
