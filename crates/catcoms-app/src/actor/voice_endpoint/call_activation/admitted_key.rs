//! Exact admitted key publication through the original actor's paid send fence.
//! This intentionally has its own typed result/publisher: extending call-selection
//! results with key receipts would charge every still-running selection task for
//! unrelated key-publication storage. Native prepays its own canonical key task.
use super::*;
use catcoms_sync::voice_v2::admitted_presence::AdmittedPresenceReceipt;

pub type VoiceAdmittedPresenceResult = Result<AdmittedPresenceReceipt, VoiceError>;

/// Native stages every accepted Work and terminal outcome in the same canonical
/// key-publication cell. Callback loss is uncertainty, not an unstarted receipt.
/// Implementations pay their Box/state separately before allocation and may not
/// enter Root, Clock, SDK, UI admission or an await from these callbacks.
pub trait VoiceAdmittedPresencePublisher: Send {
    fn accepted(&mut self, work: VoiceAdmittedPresenceWork) -> Result<(), VoiceEndpointError>;
    fn completed(
        self: Box<Self>,
        result: VoiceAdmittedPresenceResult,
        work: VoiceAdmittedPresenceWork,
    );
    fn refused(
        self: Box<Self>,
        error: VoiceEndpointError,
        work: Option<VoiceAdmittedPresenceWork>,
        payment: Option<ActivationRpcPermit>,
    );
}

#[derive(Clone)]
pub struct VoiceAdmittedPresenceWork(Arc<InvocationState<ActivationRpcPermit>>);
impl VoiceAdmittedPresenceWork {
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
impl std::fmt::Debug for VoiceAdmittedPresenceWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceAdmittedPresenceWork { .. }")
    }
}

struct Arguments {
    grant: Arc<CallGrant>,
    original: ActivationObserver,
    join: Nonce,
    point: [u8; 65],
    publisher: Option<Box<dyn VoiceAdmittedPresencePublisher>>,
    // LAST: the same retirement signal cannot precede actual argument/callback Drop.
    retirement: Option<InvocationRetirement<ActivationRpcPermit>>,
}
impl Drop for Arguments {
    fn drop(&mut self) {
        if let Some(publisher) = self.publisher.take() {
            let work = self
                .retirement
                .as_ref()
                .map(|retired| VoiceAdmittedPresenceWork(retired.0.clone()));
            publisher.refused(VoiceEndpointError::Stopped, work, None);
        }
    }
}
pub(in crate::actor::voice_endpoint) struct Submission(Paid<Arguments, VoiceAdmittedPresenceWork>);
type Preparing = Paid<Arguments, Option<ActivationRpcPermit>>;

pub(super) fn apply<T: MeshTransport, R: CryptoRngCore>(
    mut submission: Submission,
    server: &mut Server<T, R>,
) {
    let publisher = submission
        .0
        .arguments
        .publisher
        .take()
        .expect("original key publisher");
    // Taking BEFORE Root entry makes callback/actor unwind ambiguous. Only the
    // actual Root can construct the receipt, after its exact original installation.
    let result = server.sync.start_admitted_voice_presence(
        submission.0.arguments.grant.clone(),
        submission.0.arguments.original.clone(),
        submission.0.arguments.join,
        submission.0.arguments.point,
    );
    publisher.completed(result, submission.0.payment.clone());
}

impl VoiceActivationControl {
    /// Queue the original native-frozen join/public point. This grants no legacy
    /// packet ledger, agreement, Ready or RTC authority. The final Root turn must
    /// independently revalidate current actor/epoch/selected admitted activation.
    pub fn try_queue_voice_admitted_presence(
        &self,
        grant: Arc<CallGrant>,
        original: ActivationObserver,
        join: Nonce,
        point: [u8; 65],
        payment: ActivationRpcPermit,
        publisher: Box<dyn VoiceAdmittedPresencePublisher>,
    ) -> Result<VoiceAdmittedPresenceWork, VoiceEndpointError> {
        let mut preparing = Preparing {
            arguments: Arguments {
                grant,
                original,
                join,
                point,
                publisher: Some(publisher),
                retirement: None,
            },
            payment: Some(payment),
        };
        let original = &preparing.arguments.original;
        let error =
            if !preparing.payment.as_ref().unwrap().matches(original) || !self.matches(original) {
                Some(VoiceEndpointError::Protocol(VoiceError::Scope))
            } else if !original.is_active() || !original.matches_grant(&preparing.arguments.grant) {
                Some(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
            } else if voice_admitted_presence_rpc_bytes() > ACTIVATION_RPC_BYTES {
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
        let work = VoiceAdmittedPresenceWork(Arc::new(InvocationState::new(
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
                super::super::Command::CallActivation(Command::AdmittedPresence(Submission(
                    Paid {
                        arguments: preparing.arguments,
                        payment: work.clone(),
                    },
                ))),
            )),
        };
        {
            let open = owner.owner.send.lock().unwrap_or_else(|e| e.into_inner());
            if *open {
                // Same sealed actual-Actor receiver as call selection. Allocation,
                // accepted callback and every refusal/destructor stay OUTSIDE this
                // mutex; retirement closes it before receiver/control destruction.
                permit.send(envelope);
                return Ok(work);
            }
        }
        drop(permit);
        drop(envelope);
        Err(VoiceEndpointError::Stopped)
    }
}

/// Complete actor-side command/result/preparation graph. The native canonical
/// publisher, key task and SDK owners require their separate measured payments.
pub const fn voice_admitted_presence_rpc_bytes() -> usize {
    std::mem::size_of::<Envelope>()
        + std::mem::size_of::<Command>()
        + std::mem::size_of::<Preparing>()
        + std::mem::size_of::<(usize, usize, InvocationState<ActivationRpcPermit>)>()
        + 3 * std::mem::size_of::<VoiceAdmittedPresenceWork>()
        + std::mem::size_of::<VoiceAdmittedPresenceResult>()
        + std::mem::size_of::<(usize, usize, ActivationRpcPermit)>()
        + 2 * std::mem::size_of::<VoiceActivationControl>()
        + catcoms_sync::voice_v2::runtime::admitted_epoch_wake_bytes()
        + 512
}

#[cfg(test)]
mod tests;
