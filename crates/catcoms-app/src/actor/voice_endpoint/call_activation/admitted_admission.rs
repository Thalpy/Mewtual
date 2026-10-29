//! Exact original admitted neutral preparation on the sealed Actor control.
//! A native observer cannot own the result or replace a lost registration mint.
use super::*;
use catcoms_sync::voice_v2::admission::{AdmissionRpcPermit, ADMISSION_RPC_BYTES};
use catcoms_sync::voice_v2::admitted_admission::{
    AdmittedAdmissionObserver, AdmittedAdmissionPrepared,
};

pub type VoiceAdmittedAdmissionResult = Result<AdmittedAdmissionPrepared, VoiceError>;

/// Concrete native implementations prepay their own cell/Box and stage the WHOLE
/// affine result, including a registration whose later capacity reservation failed.
/// Callbacks run outside Actor/Group/Policy/send guards and may not call Root,
/// Clock, UI admission or an await. Positive attachment follows actual retirement.
/// An entered/panicking publisher Drop is UNKNOWN, never an unstarted refusal.
pub trait VoiceAdmittedAdmissionPublisher: Send {
    fn accepted(&mut self, work: VoiceAdmittedAdmissionWork) -> Result<(), VoiceEndpointError>;
    fn completed(
        self: Box<Self>,
        result: VoiceAdmittedAdmissionResult,
        work: VoiceAdmittedAdmissionWork,
    );
    fn refused(
        self: Box<Self>,
        error: VoiceEndpointError,
        work: Option<VoiceAdmittedAdmissionWork>,
        payment: Option<AdmissionRpcPermit>,
    );
}

#[derive(Clone)]
pub struct VoiceAdmittedAdmissionWork(Arc<InvocationState<AdmissionRpcPermit>>);
impl VoiceAdmittedAdmissionWork {
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
impl std::fmt::Debug for VoiceAdmittedAdmissionWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceAdmittedAdmissionWork { .. }")
    }
}

struct Arguments {
    grant: Arc<CallGrant>,
    activation: ActivationObserver,
    original: Option<AdmittedAdmissionObserver>,
    generation: u64,
    publisher: Option<Box<dyn VoiceAdmittedAdmissionPublisher>>,
    // LAST: original arguments/callback destruction precedes actual retirement.
    retirement: Option<InvocationRetirement<AdmissionRpcPermit>>,
}
impl Drop for Arguments {
    fn drop(&mut self) {
        if let Some(publisher) = self.publisher.take() {
            let work = self
                .retirement
                .as_ref()
                .map(|retired| VoiceAdmittedAdmissionWork(retired.0.clone()));
            publisher.refused(VoiceEndpointError::Stopped, work, None);
        }
    }
}
pub(in crate::actor::voice_endpoint) struct Submission(Paid<Arguments, VoiceAdmittedAdmissionWork>);
type Preparing = Paid<Arguments, Option<AdmissionRpcPermit>>;

pub(super) fn apply<T: MeshTransport, R: CryptoRngCore>(
    mut submission: Submission,
    server: &mut Server<T, R>,
) {
    // Before entry: an untouched queued publisher proves unstarted. After this
    // take: callback loss/unwind is ambiguous and cannot authorize another None.
    let publisher = submission
        .0
        .arguments
        .publisher
        .take()
        .expect("original admitted preparation publisher");
    let result = server.sync.prepare_admitted_voice_admission_step(
        &submission.0.arguments.grant,
        &submission.0.arguments.activation,
        submission.0.arguments.original.as_ref(),
        submission.0.arguments.generation,
        submission.0.payment.0.payment(),
    );
    publisher.completed(result, submission.0.payment.clone());
}

impl VoiceActivationControl {
    /// Nonblocking actual-Actor queue admission. Native must install canonical
    /// Pending before calling. Work return alone cannot recover a lost first
    /// registration: the future native Slot must retain the staged outcome and
    /// must not retry original=None merely because an observer disappeared.
    pub fn try_queue_voice_admitted_admission(
        &self,
        grant: Arc<CallGrant>,
        activation: ActivationObserver,
        original: Option<AdmittedAdmissionObserver>,
        generation: u64,
        payment: AdmissionRpcPermit,
        publisher: Box<dyn VoiceAdmittedAdmissionPublisher>,
    ) -> Result<VoiceAdmittedAdmissionWork, VoiceEndpointError> {
        let mut preparing = Preparing {
            arguments: Arguments {
                grant,
                activation,
                original,
                generation,
                publisher: Some(publisher),
                retirement: None,
            },
            payment: Some(payment),
        };
        let actual = &preparing.arguments.activation;
        let error = if generation == 0 {
            Some(VoiceEndpointError::Protocol(VoiceError::Bounds))
        } else if !self.matches(actual)
            || !preparing
                .payment
                .as_ref()
                .unwrap()
                .matches_activation(actual)
        {
            Some(VoiceEndpointError::Protocol(VoiceError::Scope))
        } else if !actual.is_active() || !actual.matches_grant(&preparing.arguments.grant) {
            Some(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
        } else if voice_admitted_admission_rpc_bytes() > ADMISSION_RPC_BYTES {
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
        let work = VoiceAdmittedAdmissionWork(Arc::new(InvocationState::new(
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
                super::super::Command::CallActivation(Command::AdmittedAdmission(Submission(
                    Paid {
                        arguments: preparing.arguments,
                        payment: work.clone(),
                    },
                ))),
            )),
        };
        {
            let open = owner
                .owner
                .send
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if *open {
                permit.send(envelope);
                return Ok(work);
            }
        }
        drop(permit);
        drop(envelope); // real unstarted destruction/callback OUTSIDE the send guard
        Err(VoiceEndpointError::Stopped)
    }
}

/// Complete fixed queue/preparation/outcome graph, before any concrete native
/// publisher. Native adds its actual Box/cell graph against the SAME4KiB fee.
pub const fn voice_admitted_admission_rpc_bytes() -> usize {
    std::mem::size_of::<Envelope>()
        + std::mem::size_of::<Submission>()
        + std::mem::size_of::<Preparing>()
        + std::mem::size_of::<(usize, usize, InvocationState<AdmissionRpcPermit>)>()
        + 3 * std::mem::size_of::<VoiceAdmittedAdmissionWork>()
        + std::mem::size_of::<VoiceAdmittedAdmissionResult>()
        + std::mem::size_of::<(usize, usize, AdmissionRpcPermit)>()
        + 2 * std::mem::size_of::<VoiceActivationControl>()
        // Root reconciles real epoch changes before preparing the registration;
        // deferred child wakes coexist with these original Actor arguments.
        + catcoms_sync::voice_v2::runtime::admitted_epoch_wake_bytes()
        + 512 // same original sender/control/refusal-return envelope as key publication
}
