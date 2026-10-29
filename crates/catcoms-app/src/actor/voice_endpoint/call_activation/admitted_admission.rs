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

/// Every alias retains the SAME plain permit outside the lifecycle allocation.
/// No raw Arc/Weak escapes. Concurrent final owners all use `into_inner`, freeing
/// the funding header before the actual permit/Credit can return to its pool.
struct FundingOwner(Option<Arc<AdmissionRpcPermit>>);
impl Clone for FundingOwner {
    fn clone(&self) -> Self {
        Self(Some(
            self.0.as_ref().expect("original admission funding").clone(),
        ))
    }
}
impl Drop for FundingOwner {
    fn drop(&mut self) {
        if let Some(permit) = self.0.take().and_then(Arc::into_inner) {
            drop(permit);
        }
    }
}
#[derive(Clone)]
pub struct VoiceAdmittedAdmissionWork {
    state: Option<Arc<InvocationState<()>>>,
    funding: FundingOwner,
}
impl Drop for VoiceAdmittedAdmissionWork {
    fn drop(&mut self) {
        drop(self.state.take());
    }
}
impl VoiceAdmittedAdmissionWork {
    pub fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(self.state.as_ref().unwrap(), other.state.as_ref().unwrap())
    }
    pub fn is_retired(&self) -> bool {
        self.state.as_ref().unwrap().is_retired()
    }
    pub async fn wait_retired(&self) {
        self.state.as_ref().unwrap().wait_retired().await;
    }
    fn payment(&self) -> &AdmissionRpcPermit {
        self.funding
            .0
            .as_deref()
            .expect("original admission payment")
    }
}
struct Retirement {
    marker: Option<InvocationRetirement<()>>,
    // LAST: marker destruction/notification and its Arc tail precede payment.
    funding: FundingOwner,
}
impl Drop for Retirement {
    fn drop(&mut self) {
        drop(self.marker.take());
    }
}
impl Retirement {
    fn work(&self) -> VoiceAdmittedAdmissionWork {
        VoiceAdmittedAdmissionWork {
            state: Some(self.marker.as_ref().unwrap().0.clone()),
            funding: self.funding.clone(),
        }
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
    retirement: Option<Retirement>,
}
impl Drop for Arguments {
    fn drop(&mut self) {
        if let Some(publisher) = self.publisher.take() {
            let work = self.retirement.as_ref().map(Retirement::work);
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
        submission.0.payment.payment(),
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
        let funding = FundingOwner(Some(Arc::new(preparing.payment.take().unwrap())));
        let work = VoiceAdmittedAdmissionWork {
            state: Some(Arc::new(InvocationState::new(()))),
            funding: funding.clone(),
        };
        preparing.arguments.retirement = Some(Retirement {
            marker: Some(InvocationRetirement(work.state.as_ref().unwrap().clone())),
            funding,
        });
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
        // Keep the earlier larger State<P> allowance after moving P outside;
        // add all new wrappers without trimming original inventory terms.
        + 2 * std::mem::size_of::<FundingOwner>()
        + std::mem::size_of::<Retirement>()
        + 3 * std::mem::size_of::<VoiceAdmittedAdmissionWork>()
        + std::mem::size_of::<VoiceAdmittedAdmissionResult>()
        + std::mem::size_of::<(usize, usize, AdmissionRpcPermit)>()
        + 2 * std::mem::size_of::<VoiceActivationControl>()
        // Root reconciles real epoch changes before preparing the registration;
        // deferred child wakes coexist with these original Actor arguments.
        + catcoms_sync::voice_v2::runtime::admitted_epoch_wake_bytes()
        + 512 // same original sender/control/refusal-return envelope as key publication
}
