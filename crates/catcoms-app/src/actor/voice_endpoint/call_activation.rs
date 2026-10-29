//! Paid inactive call activation. Every result is staged synchronously in the
//! original native cell; an invoke waiter never owns grant selection or cleanup.
use super::sdp_input::{InvocationRetirement, InvocationState, Paid};
use super::*;
use catcoms_sync::voice_v2::call_activation::{
    ActivationObserver, ActivationRpcPermit, PreparedAdmittedCall, RootSelectedCall,
    ACTIVATION_RPC_BYTES,
};
mod control;
pub use control::{
    control_layout_bytes, PreparedVoiceCall, VoiceActivationControl, VOICE_ACTIVATION_CONTROL_BYTES,
};
pub(in crate::actor) use control::{ActorControl, ActorInbox};

#[derive(Debug)]
pub enum VoiceCallActivationResult {
    Selected(Result<RootSelectedCall, VoiceError>),
    Cleared(Result<(), VoiceError>),
}
/// Concrete native implementation must pay its full Box/cell/callback graph before
/// allocation. Completed runs after Root releases Group/Policy; no UI guard, Clock,
/// Root factory or await may occur here. Native's already-held UI guard belongs to
/// its canonical task, not this callback or an observing oneshot.
pub trait VoiceCallActivationPublisher: Send {
    fn accepted(&mut self, work: VoiceCallActivationWork) -> Result<(), VoiceEndpointError>;
    fn completed(self: Box<Self>, result: VoiceCallActivationResult, work: VoiceCallActivationWork);
    /// Only this path proves no Root entry. A pre-enqueue refusal RETURNS the SAME
    /// affine payment; an accepted command instead retains it in its original Work.
    fn refused(
        self: Box<Self>,
        error: VoiceEndpointError,
        work: Option<VoiceCallActivationWork>,
        payment: Option<ActivationRpcPermit>,
    );
}
#[derive(Clone)]
pub struct VoiceCallActivationWork(Arc<InvocationState<ActivationRpcPermit>>);
impl VoiceCallActivationWork {
    pub fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn is_retired(&self) -> bool {
        self.0.is_retired()
    }
    pub async fn wait_retired(&self) {
        self.0.wait_retired().await;
    }
}
impl std::fmt::Debug for VoiceCallActivationWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceCallActivationWork { .. }")
    }
}

enum Action {
    Select {
        grant: Arc<CallGrant>,
        original: ActivationObserver,
    },
    Clear {
        original: ActivationObserver,
    },
}
impl Action {
    fn original(&self) -> &ActivationObserver {
        match self {
            Self::Select { original, .. } | Self::Clear { original } => original,
        }
    }
}
pub(super) struct Arguments {
    action: Action,
    publisher: Option<Box<dyn VoiceCallActivationPublisher>>,
    // LAST argument: actual action/publisher teardown precedes retirement.
    retirement: Option<InvocationRetirement<ActivationRpcPermit>>,
}
impl Drop for Arguments {
    fn drop(&mut self) {
        if let Some(publisher) = self.publisher.take() {
            let work = self
                .retirement
                .as_ref()
                .map(|retired| VoiceCallActivationWork(retired.0.clone()));
            publisher.refused(VoiceEndpointError::Stopped, work, None);
        }
    }
}
pub(super) type Selection = Paid<Arguments, VoiceCallActivationWork>;
type Preparing = Paid<Arguments, Option<ActivationRpcPermit>>;
pub(super) enum Command {
    Selection(Selection),
    Prepare {
        room: u128,
        trace: Trace,
        reply: Reply<PreparedVoiceCall>,
        // Payment survives both caller loss and the actual sender's return. This
        // plain Arc contains no prepared call, grant, Root or native cell backedge.
        payment: Arc<ActivationRpcPermit>,
    },
}
pub(super) fn apply<T: MeshTransport, R: CryptoRngCore>(
    command: Command,
    server: &mut Server<T, R>,
    control: &mut ActorControl,
) {
    match command {
        Command::Prepare {
            room,
            trace,
            reply,
            payment,
        } => {
            if !reply.is_closed() {
                let prepared = server
                    .sync
                    .prepare_paid_admitted_voice_call(room, &payment)
                    .and_then(|prepared| control.wrap(prepared, trace));
                let _ = reply.send(prepared);
            }
            drop(payment); // after actual send/returned-output ownership handoff
        }
        Command::Selection(mut command) => {
            // Taking BEFORE entry makes entered unwind uncertain, never unstarted.
            let publisher = command
                .arguments
                .publisher
                .take()
                .expect("original call publisher");
            let result = match &command.arguments.action {
                Action::Select { grant, original } => VoiceCallActivationResult::Selected(
                    server.sync.select_admitted_voice_call(grant, original),
                ),
                Action::Clear { original } => VoiceCallActivationResult::Cleared(
                    server.sync.clear_admitted_voice_call(original),
                ),
            };
            publisher.completed(result, command.payment.clone());
        }
    }
}
impl ServerActor {
    /// Trusted inactive preparation. Its separate original payment precedes queue,
    /// oneshot and future allocation; a lost reply drops/revokes the inert pair.
    pub async fn prepare_admitted_voice_call(
        &self,
        room: u128,
        payment: ActivationRpcPermit,
    ) -> Result<PreparedVoiceCall, VoiceEndpointError> {
        let payment = Arc::new(payment);
        let held = payment.clone();
        self.voice_endpoint_request(|reply| {
            super::Command::CallActivation(Command::Prepare {
                room,
                trace: self.cmd_tx.trace,
                reply,
                payment: held,
            })
        })
        .await
    }
}
impl VoiceActivationControl {
    pub fn try_queue_voice_call_selection(
        &self,
        grant: Arc<CallGrant>,
        original: ActivationObserver,
        payment: ActivationRpcPermit,
        publisher: Box<dyn VoiceCallActivationPublisher>,
    ) -> Result<VoiceCallActivationWork, VoiceEndpointError> {
        self.queue_call_activation(Action::Select { grant, original }, payment, publisher)
    }
    /// Negative-only, bound to the original anchor. No positive grant/UI/epoch
    /// lookup is required, and this cannot select or clear a different activation.
    pub fn try_queue_voice_call_clear(
        &self,
        original: ActivationObserver,
        payment: ActivationRpcPermit,
        publisher: Box<dyn VoiceCallActivationPublisher>,
    ) -> Result<VoiceCallActivationWork, VoiceEndpointError> {
        self.queue_call_activation(Action::Clear { original }, payment, publisher)
    }
    fn queue_call_activation(
        &self,
        action: Action,
        payment: ActivationRpcPermit,
        publisher: Box<dyn VoiceCallActivationPublisher>,
    ) -> Result<VoiceCallActivationWork, VoiceEndpointError> {
        let mut preparing = Preparing {
            arguments: Arguments {
                action,
                publisher: Some(publisher),
                retirement: None,
            },
            payment: Some(payment),
        };
        let error = if !preparing
            .payment
            .as_ref()
            .unwrap()
            .matches(preparing.arguments.action.original())
            || !self.matches(preparing.arguments.action.original())
        {
            Some(VoiceEndpointError::Protocol(VoiceError::Scope))
        } else if voice_call_activation_rpc_bytes() > ACTIVATION_RPC_BYTES {
            Some(VoiceEndpointError::Protocol(VoiceError::Capacity))
        } else {
            None
        };
        if let Some(error) = error {
            preparing.arguments.publisher.take().unwrap().refused(
                error,
                None,
                preparing.payment.take(),
            );
            return Err(error);
        }
        // A real stdWeak upgrade is transient and exact. It is unavailability,
        // never proof that the original Root/residence or entered Work returned.
        let Some(owner) = self.upgrade() else {
            let error = VoiceEndpointError::Stopped;
            preparing.arguments.publisher.take().unwrap().refused(
                error,
                None,
                preparing.payment.take(),
            );
            return Err(error);
        };
        let Some(sender) = owner.owner.sender.upgrade() else {
            let error = VoiceEndpointError::Stopped;
            preparing.arguments.publisher.take().unwrap().refused(
                error,
                None,
                preparing.payment.take(),
            );
            return Err(error);
        };
        // Preserve Full's original affine-payment return before Work/accepted.
        // Permit remains outside the final fence, including on every destructor.
        let permit = match sender.try_reserve() {
            Ok(permit) => permit,
            Err(error) => {
                let error = match error {
                    mpsc::error::TrySendError::Full(()) => {
                        VoiceEndpointError::Protocol(VoiceError::Capacity)
                    }
                    mpsc::error::TrySendError::Closed(()) => VoiceEndpointError::Stopped,
                };
                preparing.arguments.publisher.take().unwrap().refused(
                    error,
                    None,
                    preparing.payment.take(),
                );
                return Err(error);
            }
        };
        let work = VoiceCallActivationWork(Arc::new(InvocationState::new(
            preparing.payment.take().unwrap(),
        )));
        preparing.arguments.retirement = Some(InvocationRetirement(work.0.clone()));
        if let Err(error) = preparing
            .arguments
            .publisher
            .as_mut()
            .unwrap()
            .accepted(work.clone())
        {
            preparing
                .arguments
                .publisher
                .take()
                .unwrap()
                .refused(error, Some(work.clone()), None);
            return Err(error);
        }
        let envelope = Envelope {
            trace: self.trace,
            command: AppCommand::VoiceEndpoint(VoiceEndpointCommand(
                super::Command::CallActivation(Command::Selection(Selection {
                    arguments: preparing.arguments,
                    payment: work.clone(),
                })),
            )),
        };
        {
            let open = owner.owner.send.lock().unwrap_or_else(|e| e.into_inner());
            if *open {
                // ONLY check+move/send under this bounded fence. The receiver is
                // private to actor::spawn/fair::next and has Tokio's sealed task
                // waker. This is not a generic arbitrary-receiver-waker contract.
                permit.send(envelope);
                return Ok(work);
            }
        }
        // Retirement won after accepted(). No Root entry happened, but the SAME
        // actual Work/arguments must retire; its payment is no longer affine.
        drop(permit);
        drop(envelope); // Arguments::drop stages Stopped outside ALL send guards
        Err(VoiceEndpointError::Stopped)
    }
}
/// Concrete queue/preparation/callback/output/retirement inventory. Native measures
/// its actual Publisher/canonical cell/future separately before constructing them.
pub const fn voice_call_activation_rpc_bytes() -> usize {
    std::mem::size_of::<Envelope>()
        + std::mem::size_of::<Command>()
        + std::mem::size_of::<Preparing>()
        + std::mem::size_of::<(usize, usize, InvocationState<ActivationRpcPermit>)>()
        + 3 * std::mem::size_of::<VoiceCallActivationWork>()
        + std::mem::size_of::<VoiceCallActivationResult>()
        + std::mem::size_of::<(usize, usize, ActivationRpcPermit)>()
        + 2 * std::mem::size_of::<VoiceActivationControl>()
        + catcoms_sync::voice_v2::runtime::admitted_epoch_wake_bytes()
        + 512 // prepare oneshot/queue permits, alignment and actual callback return frames
}

#[cfg(test)]
mod tests;
