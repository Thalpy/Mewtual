//! Separately paid original SDP publication worker, for the staged HTTP adapter. A
//! waiter is observation only: dropping it detaches work, never aborts a queued command.
//! The affine Writer retires before Registration or the final bound IO/payment can wake
//! cleanup. Root repeats its own current authority; this module enables no listener opcode.
use super::bootstrap::Route;
use super::channel::{PhysicalHold, Session};
use super::context::Context;
use super::factory::{self, Refused};
use super::ledger::{ResolvedRow, SdpPublicationTarget, SdpRegistration, SdpSubmissionOwners};
use super::sdp_publication::{self, Publication};
use catcoms_app::{
    voice_sdp_rpc_bytes, ServerActor, VoiceSdpBody, VoiceSdpPending, VoiceSdpPublication,
    VoiceSdpRecordedInput, VOICE_SDP_RPC_METADATA_BYTES,
};
use catcoms_sync::voice_v2::chain::sdp_handoff::{Copy, NativeRowPurpose, TaskCopy};
use catcoms_sync::voice_v2::crypto_actions::CryptoAction;
use catcoms_sync::voice_v2::sdp::publication::Writer;
use catcoms_sync::voice_v2::sdp::{InitialSdpIntent, SdpDelivery, SdpPublicationAdmission};
use catcoms_sync::voice_v2::{CallGrant, VoiceError};
use std::sync::Arc;

/// Only trusted native producers can supply the actual original paid allocation or the
/// actual accepted delivery. HTTP decoding still must check full encrypted row bindings.
pub(super) enum Payload {
    Seal {
        body: VoiceSdpBody,
        delivery: Option<Arc<SdpDelivery>>,
    },
    Open,
}
enum Input {
    Seal {
        body: VoiceSdpBody,
        delivery: Option<Arc<SdpDelivery>>,
    },
    Open(Option<Copy>),
}
/// Field order applies to local refusal and never-polled futures as well. Publication's
/// independent TaskCopy is LAST; it must not retain this preparation/body/Registration.
pub(super) struct Preparation {
    writer: Option<Writer>,
    registration: SdpRegistration,
    input: Input,
    intent: Arc<InitialSdpIntent>,
    handshake: [u8; 32],
    publication: Publication,
}
/// Refusal/unwind order exists BEFORE the first fallible snapshot or copy reservation,
/// including Open's partially built metadata. Empty slots own no allocation. Registration
/// precedes every body/physical/payment owner, and Writer always precedes Registration.
struct Construction {
    writer: Option<Writer>,
    registration: Option<SdpRegistration>,
    payload: Option<Payload>,
    input: Option<Input>,
    owners: Option<SdpSubmissionOwners>,
    publication: Option<Publication>,
    physical: Option<PhysicalHold>,
    task: Option<TaskCopy>,
}
pub(super) struct Completion {
    result: Result<ResolvedRow, Refused>,
    _publication: Arc<Publication>,
}
impl Completion {
    pub(super) fn result(&self) -> Result<&ResolvedRow, Refused> {
        self.result.as_ref().map_err(|error| *error)
    }
    /// Consume actual completed task/publication custody before the HTTP caller observes
    /// Root history or reacquires UI. The affine ResolvedRow is moved, never cloned into
    /// an unpaid extra handle. Its own original fixed history funding remains intact.
    pub(super) fn into_result(self) -> Result<ResolvedRow, Refused> {
        let Self {
            result,
            _publication,
        } = self;
        drop(_publication);
        result
    }
}
/// The raw task/abort handle never escapes. Actual output retains original task funding
/// through collection, including a caller which loses the response before observing it.
pub(super) struct Waiter(tokio::task::JoinHandle<Completion>);
impl std::future::Future for Waiter {
    type Output = Result<Completion, Refused>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.get_mut().0)
            .poll(cx)
            .map(|result| result.map_err(|_| Refused::Stopped))
    }
}
/// Concrete native final fence; the original Task pays this Box in addition to the
/// existing Worker/Actor envelope. There is no channel, new Copy or Registration backedge.
struct Publisher {
    context: Context,
    route: Route,
    target: SdpPublicationTarget,
    // LAST. Original target/native metadata retires before its actual Task/physical owner.
    publication: Arc<Publication>,
}
impl VoiceSdpPublication for Publisher {
    fn publish(self: Box<Self>, step: SdpPublicationAdmission<'_>) -> Result<(), VoiceError> {
        let live = self.context.upgrade().ok_or(VoiceError::Unauthorized)?;
        let commit = live
            .commit
            .clone()
            .try_lock_owned()
            .map_err(|_| VoiceError::Capacity)?;
        factory::require_unlocked_now(&live, self.route.selected()).map_err(
            |error| match error {
                Refused::Protocol(reason) => reason,
                _ => VoiceError::Unauthorized,
            },
        )?;
        let result = {
            let authority = match live.authority.try_lock() {
                Ok(authority) => authority,
                Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
            };
            let admission =
                super::transition::Admission::new(&live, &authority, &self.route, &commit);
            self.route
                .try_with_open(|| self.target.admit(&admission, step))
        };
        // Group/Policy and every short native guard are gone before custody installation
        // or any affine/fallible owner destruction. No Clock follows successful admission.
        result?;
        self.publication.admit(commit);
        Ok(())
    }
}
struct Worker {
    writer: Option<Writer>,
    registration: SdpRegistration,
    input: Input,
    publisher: Option<Box<Publisher>>,
    actor: ServerActor,
    grant: Arc<CallGrant>,
    intent: Arc<InitialSdpIntent>,
    handshake: [u8; 32],
    #[cfg(test)]
    queued: Option<tokio::sync::oneshot::Sender<()>>,
    publication: Arc<Publication>,
}
impl Preparation {
    /// Prepay before constructing any detached worker. Original body/metadata and TaskCopy
    /// bind the SAME fixed row counter; same-Phase bytes on another counter are refused.
    /// Every fallible allocation precedes acquiring Writer. Before final UI admission this
    /// is only lifetime/payment custody, with no permission to enqueue or use an SDK.
    pub(super) fn new(
        registration: SdpRegistration,
        physical: PhysicalHold,
        session: &Arc<Session>,
        payload: Payload,
    ) -> Result<Self, VoiceError> {
        Self::new_inner(
            registration,
            physical,
            session,
            payload,
            #[cfg(test)]
            || {},
        )
    }
    #[cfg(test)]
    pub(super) fn new_after_payment(
        registration: SdpRegistration,
        physical: PhysicalHold,
        session: &Arc<Session>,
        payload: Payload,
        after_payment: impl FnOnce(),
    ) -> Result<Self, VoiceError> {
        Self::new_inner(registration, physical, session, payload, after_payment)
    }
    fn new_inner(
        registration: SdpRegistration,
        physical: PhysicalHold,
        session: &Arc<Session>,
        payload: Payload,
        #[cfg(test)] after_payment: impl FnOnce(),
    ) -> Result<Self, VoiceError> {
        let mut construction = Construction {
            writer: None,
            registration: Some(registration),
            payload: Some(payload),
            input: None,
            owners: None,
            publication: None,
            physical: Some(physical),
            task: None,
        };
        if task_layout() > sdp_publication::TASK_BYTES {
            return Err(VoiceError::Capacity);
        }
        // This small prepayment snapshot is inventoried in the ORIGINAL fixed row fee.
        // No variable allocation or detached future exists before TaskCopy is acquired.
        construction.owners = Some(
            construction
                .registration
                .as_ref()
                .expect("original worker")
                .submission_owners(session.identity())?,
        );
        let owners = construction.owners.as_ref().expect("original provenance");
        if (owners.funding.purpose() == NativeRowPurpose::Seal)
            != matches!(construction.payload, Some(Payload::Seal { .. }))
        {
            return Err(VoiceError::Unauthorized);
        }
        construction.task = Some(owners.phase.task_copy(sdp_publication::TASK_BYTES)?);
        construction
            .task
            .as_mut()
            .expect("prepaid native task")
            .bind_native_history(owners.funding.clone())?;
        construction.publication = Some(Publication::retaining(
            construction
                .physical
                .take()
                .expect("original physical position"),
            construction
                .task
                .take()
                .expect("original native task payment"),
        ));
        construction
            .publication
            .as_ref()
            .expect("retained publication")
            .validate(session, &owners.intent)?;
        #[cfg(test)]
        after_payment(); // fixture barrier outside every native/Root/Phase guard
        match construction.payload.take().expect("one original payload") {
            Payload::Seal { body, delivery } => {
                // Install all affine owners before validating them. A refusal cannot
                // destroy publication/payment while Registration still reports a worker.
                construction.input = Some(Input::Seal { body, delivery });
                let Some(Input::Seal { body, delivery }) = &construction.input else {
                    unreachable!()
                };
                if !body.uses_native_history(&owners.intent, &owners.funding)
                    || owners.intent.local_is_low() == delivery.is_some()
                {
                    return Err(VoiceError::Unauthorized);
                }
            }
            Payload::Open => {
                construction.input = Some(Input::Open(Some(
                    owners.phase.copy(VOICE_SDP_RPC_METADATA_BYTES)?,
                )));
                let Some(Input::Open(Some(copy))) = &mut construction.input else {
                    unreachable!()
                };
                copy.bind_native_history(owners.funding.clone())?;
            }
        }
        construction.writer = Some(
            construction
                .registration
                .as_ref()
                .expect("original worker")
                .publication_writer()?,
        );
        let intent = owners.intent.clone();
        let handshake = owners.handshake;
        Ok(Self {
            writer: construction.writer.take(),
            registration: construction.registration.take().expect("original worker"),
            input: construction.input.take().expect("original paid input"),
            intent,
            handshake,
            publication: construction
                .publication
                .take()
                .expect("original native task"),
        })
    }

    /// Final nonblocking native admission. Call the existing Actor preflight before this
    /// synchronous function; no broad mounted context crosses the worker's Actor wait.
    /// The short PreRoot handoff fence is released BEFORE enqueue or Clock. The actual
    /// final publisher later installs UI custody inside the scoped Actor callback.
    pub(super) fn launch(self, context: &Context, route: &Route) -> Result<Waiter, Refused> {
        self.launch_inner(
            context,
            route,
            #[cfg(test)]
            None,
        )
    }
    /// Fixture-only observation of actual mailbox admission. It supplies no authority,
    /// payment, fake current state or replacement action, and never reaches HTTP/IPC.
    #[cfg(test)]
    pub(super) fn launch_with_queue_barrier(
        self,
        context: &Context,
        route: &Route,
        queued: tokio::sync::oneshot::Sender<()>,
    ) -> Result<Waiter, Refused> {
        self.launch_inner(context, route, Some(queued))
    }
    fn launch_inner(
        self,
        context: &Context,
        route: &Route,
        #[cfg(test)] queued: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<Waiter, Refused> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| Refused::Stopped)?;
        let (actor, grant) = factory::native::observation_actor(context, route)?;
        let live = context.upgrade().ok_or(Refused::Closed)?;
        let clock = live
            .authority
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .runtime()
            .clock();
        // Clock callbacks precede all UI/Coordinator/Index/State final guards.
        let wall = clock.now_ms();
        let mono = clock.monotonic_ms();
        drop(clock); // no launch-only owner overlaps the running worker/spawn graph
        let commit = live
            .commit
            .clone()
            .try_lock_owned()
            .map_err(|_| VoiceError::Capacity)?;
        factory::require_unlocked_now(&live, route.selected())?;
        {
            let authority = match live.authority.try_lock() {
                Ok(authority) => authority,
                Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity.into()),
            };
            let admission = super::transition::Admission::new(&live, &authority, route, &commit);
            route.try_with_open(|| {
                self.registration.mark_handoff(
                    &admission,
                    self.writer.as_ref().expect("original affine Writer"),
                    wall,
                    mono,
                )
            })?;
        }
        drop(live);
        drop(commit);
        let Self {
            writer,
            registration,
            input,
            intent,
            handshake,
            publication,
        } = self;
        // Restore the ordered affine owner BEFORE any further fallible call. Independent
        // locals here could return TaskCopy ahead of a still-reserved Writer on failure.
        let mut worker = Box::new(Worker {
            writer,
            registration,
            input,
            publisher: None,
            actor,
            grant,
            intent,
            handshake,
            #[cfg(test)]
            queued,
            publication: Arc::new(publication),
        });
        worker.publisher = Some(Box::new(Publisher {
            context: context.clone(),
            route: route.clone(),
            target: worker
                .registration
                .publication_target(route.session().identity())?,
            publication: worker.publication.clone(),
        }));
        // Shared work initially retains physical/payment lifetime custody with NO UI
        // guard. The actual Actor's scoped publication tail installs UI only on success. The original
        // TaskCopy cannot receive this keepalive, so no self-funding cycle can form.
        match &mut worker.input {
            Input::Seal { body, .. } => body.attach_publication(worker.publication.clone())?,
            Input::Open(copy) => copy
                .as_mut()
                .expect("original metadata")
                .attach_keepalive(worker.publication.clone())?,
        }
        let future = run(worker);
        if worker_layout(std::mem::size_of_val(&future)) > sdp_publication::TASK_BYTES {
            // Root has not been entered. Ordered future destruction publishes NeverEntered
            // before worker/IO notification; exact retry retains this same row and slot.
            return Err(VoiceError::Capacity.into());
        }
        Ok(Waiter(runtime.spawn(future)))
    }
}
async fn run(mut worker: Box<Worker>) -> Completion {
    // Construct one original request synchronously, then await ONE sender type. The
    // prior three awaited alternatives retained avoidable branch storage in this Task.
    let writer = worker
        .writer
        .take()
        .expect("one original publication invocation");
    let publisher = worker.publisher.take().expect("one original final fence");
    let input = match &mut worker.input {
        Input::Seal {
            body,
            delivery: None,
        } => VoiceSdpRecordedInput::Offer(body),
        Input::Seal {
            body,
            delivery: Some(delivery),
        } => VoiceSdpRecordedInput::Answer {
            body,
            delivery: delivery.clone(),
        },
        Input::Open(copy) => {
            VoiceSdpRecordedInput::Open(copy.take().expect("one original metadata Copy"))
        }
    };
    let queued = worker.actor.queue_voice_sdp_recorded_scoped(
        worker.grant.clone(),
        worker.handshake,
        worker.intent.clone(),
        input,
        writer,
        publisher,
    );
    let pending = match checked_queue(queued) {
        Ok(future) => future.await.map_err(Refused::from),
        Err(error) => Err(error.into()),
    };
    let result = match pending {
        Err(error) => Err(error),
        Ok(pending) => {
            #[cfg(test)]
            if let Some(queued) = worker.queued.take() {
                let _ = queued.send(());
            }
            match pending.complete().await {
                Err(error) => Err(error.into()),
                Ok(completion) => {
                    // Capture the SAME actual Root Action BEFORE any later attachment or
                    // observation, including after Return. Normal errors remain recorded.
                    let result = match completion.result() {
                        Ok(action) => worker
                            .registration
                            .capture(action)
                            .and_then(|()| worker.registration.attach_published())
                            .map(|()| worker.registration.resolved())
                            .map_err(Refused::from),
                        Err(error) => Err(error.into()),
                    };
                    drop(completion);
                    result
                }
            }
        }
    };
    drop(worker.writer.take());
    // Actor completion and canonical publication capture are finished. Root's actual
    // Action owns its independently paid input; retaining the old native request bytes
    // in a metadata-only output would needlessly hold another buffer/IO contribution.
    // Destroy them now, while this SAME publication still pays the final task/output.
    drop(std::mem::replace(&mut worker.input, Input::Open(None)));
    // A delivered RPC completion can beat the sender's synchronous send/last Work Drop.
    // That Work still retains THIS actual publication UI guard. Keep Registration's
    // factory exclusion and this Session's sole physical request while waiting for all
    // other SAME-counter IO, without Clock or another Actor query under the held UI gate.
    // Only then may into_result release the final publication before historical Clock.
    let result = match worker.publication.wait_other_io_collected().await {
        Ok(()) => result,
        Err(error) => Err(error.into()),
    };
    Completion {
        result,
        // Keep Worker intact: its original fields drop before publication on unwind too.
        _publication: worker.publication.clone(),
    }
}
fn checked_queue<F>(future: F) -> Result<F, VoiceError>
where
    F: std::future::Future<
        Output = Result<VoiceSdpPending<Arc<CryptoAction>>, catcoms_app::VoiceEndpointError>,
    >,
{
    if voice_sdp_rpc_bytes(std::mem::size_of_val(&future)) > VOICE_SDP_RPC_METADATA_BYTES {
        return Err(VoiceError::Capacity);
    }
    Ok(future)
}
// Synchronous constructor/final-admission ownership differs from the running task.
// These fixed graphs do not coexist with the queued run future. Count actual wrappers
// explicitly; keep pinned Tokio task overhead separate from native constructor storage.
fn construction_layout() -> usize {
    std::mem::size_of::<Construction>()
        + std::mem::size_of::<Preparation>()
        + std::mem::size_of::<SdpSubmissionOwners>()
        + std::mem::size_of::<Payload>()
        + std::mem::size_of::<Input>()
        + 4 * std::mem::size_of::<usize>() // borrowed arguments and constructor discriminants
}
fn launch_layout() -> usize {
    std::mem::size_of::<Preparation>()
        + std::mem::size_of::<Box<Worker>>()
        + std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<crate::UiSessionGate<'static>>()
        + std::mem::size_of::<super::transition::Admission<'static>>()
        + std::mem::size_of::<std::sync::MutexGuard<'static, crate::voice_authority::Coordinator>>()
        + std::mem::size_of::<tokio::sync::OwnedMutexGuard<()>>()
        + std::mem::size_of::<tokio::runtime::Handle>()
        + std::mem::size_of::<ServerActor>()
        + std::mem::size_of::<Arc<CallGrant>>()
        + std::mem::size_of::<Arc<dyn catcoms_rt::Clock>>()
        + std::mem::size_of::<Arc<tokio::sync::Mutex<()>>>()
        + 8 * std::mem::size_of::<usize>() // borrowed native arguments, clock samples and guards
}
fn worker_layout(future_bytes: usize) -> usize {
    let running = future_bytes
        + std::mem::size_of::<tokio::runtime::Handle>() // launch owns it during spawn
        + std::mem::size_of::<Completion>()
        + std::mem::size_of::<Waiter>()
        + 512; // pinned Tokio task allocation/handles; no native constructor allowance
    let publisher = std::mem::size_of::<Publisher>()
        + std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<super::transition::Admission<'static>>()
        + std::mem::size_of::<crate::UiSessionGate<'static>>()
        + std::mem::size_of::<std::sync::MutexGuard<'static, crate::voice_authority::Coordinator>>(
        )
        + 3 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + std::mem::size_of::<tokio::sync::OwnedMutexGuard<()>>()
        + 128; // borrowed scoped tail, selectors and short guard bookkeeping
    sdp_publication::FIXED_BYTES
        + std::mem::size_of::<Worker>() // sole Box backing, retained through the task
        + publisher
        + running.max(construction_layout()).max(launch_layout())
}

/// Infer the original spawned future without constructing any Worker or allocation.
/// Refuse a layout change before Task/Publisher/Worker construction, not after payment.
fn task_layout() -> usize {
    fn measured<F: std::future::Future<Output = Completion>>(_: fn(Box<Worker>) -> F) -> usize {
        worker_layout(std::mem::size_of::<F>())
    }
    measured(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_sdp_submission_task_graph_has_no_new_payment_pool() {
        // Infer the ACTUAL future type without constructing a fake Worker. This includes
        // all awaited recorded queue alternatives and the final output/custodian graph.
        fn measured<F: std::future::Future<Output = Completion>>(_: fn(Box<Worker>) -> F) -> usize {
            worker_layout(std::mem::size_of::<F>())
        }
        let bytes = measured(run);
        eprintln!(
            "SDP worker graph {bytes}, Worker {}, Publisher {}, publication {}",
            std::mem::size_of::<Worker>(),
            std::mem::size_of::<Publisher>(),
            sdp_publication::FIXED_BYTES
        );
        assert!(
            bytes <= sdp_publication::TASK_BYTES,
            "complete original SDP worker graph needs {bytes}; constructor {}, launch {}",
            construction_layout(),
            launch_layout()
        );
    }
}
