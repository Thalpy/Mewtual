//! Paid, synchronous staging of the original Root admission outcome. Staging is
//! quarantine custody, never native permission. No oneshot waiter can lose a minted
//! registration and cause the same physical Session to mint another incarnation.
use super::sdp_input::{InvocationRetirement, InvocationState, Paid};
use super::*;
use catcoms_sync::voice_v2::admission::{
    AdmissionPrepared, AdmissionRpcPermit, AdmissionSessionObserver, ADMISSION_RPC_BYTES,
};

/// Native implements one concrete, prepaid publisher for the canonical physical
/// Session/invocation allocation. `completed` must stage the SAME affine outcome even
/// after Cancel/End; it grants no Ready or effect. It may not await, call Root/Clock,
/// or acquire UI commit. Positive native attachment happens later, after retirement.
///
/// `refused` means DEFINITELY NEVER ENTERED Root. The adapter's own Drop must instead
/// quarantine an entered/panicking/unstaged invocation as uncertain, never absent.
/// Complete native Box/callback/output ownership needs concrete layout validation
/// BEFORE allocation; a trait-reported size is not that proof.
pub trait VoiceAdmissionPublisher: Send {
    /// Install exact custody in canonical Pending BEFORE enqueue can wake Root or
    /// the observing worker disappears. This grants no effect. Err is unstarted.
    fn accepted(&mut self, work: VoiceAdmissionWork) -> Result<(), VoiceEndpointError>;
    fn completed(
        self: Box<Self>,
        result: Result<AdmissionPrepared, VoiceError>,
        work: VoiceAdmissionWork,
    );
    /// Some(work) covers queued/prepared-command destruction. Callback return is
    /// BEFORE actual InvocationRetirement; None is a pre-enqueue refusal.
    fn refused(self: Box<Self>, error: VoiceEndpointError, work: Option<VoiceAdmissionWork>);
}

/// Exact queue/callback custody. The fixed fee follows actual commands, outputs and
/// observers, with no Route/Row/publisher backedge. Retirement means actual callback
/// teardown, not absence, currentness, capacity collection or permission to remint.
#[derive(Clone)]
pub struct VoiceAdmissionWork(Arc<InvocationState<AdmissionRpcPermit>>);
impl VoiceAdmissionWork {
    pub fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub async fn wait_retired(&self) {
        self.0.wait_retired().await;
    }
    pub fn is_retired(&self) -> bool {
        self.0.is_retired()
    }
}
impl std::fmt::Debug for VoiceAdmissionWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceAdmissionWork { .. }")
    }
}

pub(super) struct Arguments {
    grant: Arc<CallGrant>,
    original: Option<AdmissionSessionObserver>,
    generation: u64,
    publisher: Option<Box<dyn VoiceAdmissionPublisher>>,
    // LAST argument: every callback/original owner returns before this signal.
    // Paid below continues the real contribution through this destructor too.
    retirement: Option<InvocationRetirement<AdmissionRpcPermit>>,
}
impl Drop for Arguments {
    fn drop(&mut self) {
        // Only an untouched queued publisher proves no Root entry. apply removes it
        // into an affine local BEFORE invoking either Root factory. Unwind after that
        // point must use the concrete native publisher's uncertain Drop path instead.
        if let Some(publisher) = self.publisher.take() {
            let work = self
                .retirement
                .as_ref()
                .map(|retired| VoiceAdmissionWork(retired.0.clone()));
            publisher.refused(VoiceEndpointError::Stopped, work);
        }
    }
}
pub(super) type Command = Paid<Arguments, VoiceAdmissionWork>;
type Preparing = Paid<Arguments, Option<AdmissionRpcPermit>>;

pub(super) fn apply<T: MeshTransport, R: CryptoRngCore>(
    mut command: Command,
    server: &mut Server<T, R>,
) {
    let publisher = command
        .arguments
        .publisher
        .take()
        .expect("one original admission publisher");
    let result = server.sync.prepare_voice_admission_step(
        &command.arguments.grant,
        command.arguments.original.as_ref(),
        command.arguments.generation,
        command.payment.0.payment(),
    );
    // Root has released Group/Policy guards. The native adapter stages the original
    // result synchronously; command remains intact and paid through callback return.
    publisher.completed(result, command.payment.clone());
}

impl ServerActor {
    /// Nonblocking original queue admission. Full mailbox is proven unstarted and
    /// may retry the SAME native generation. An accepted invocation must recover its
    /// original staged outcome; no observing waiter, cancellation or retirement bool
    /// can authorize a fresh registration factory. Payment precedes native Box creation.
    pub fn try_queue_voice_admission(
        &self,
        grant: Arc<CallGrant>,
        original: Option<AdmissionSessionObserver>,
        generation: u64,
        payment: AdmissionRpcPermit,
        publisher: Box<dyn VoiceAdmissionPublisher>,
    ) -> Result<VoiceAdmissionWork, VoiceEndpointError> {
        let mut preparing = Preparing {
            arguments: Arguments {
                grant,
                original,
                generation,
                publisher: Some(publisher),
                retirement: None,
            },
            payment: Some(payment),
        };
        let error = if generation == 0 {
            Some(VoiceEndpointError::Protocol(VoiceError::Bounds))
        } else if preparing
            .payment
            .as_ref()
            .expect("one original admission payment")
            .scope()
            != preparing.arguments.grant.scope()
        {
            Some(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
        } else if voice_admission_rpc_bytes() > ADMISSION_RPC_BYTES {
            Some(VoiceEndpointError::Protocol(VoiceError::Capacity))
        } else {
            None
        };
        if let Some(error) = error {
            preparing
                .arguments
                .publisher
                .take()
                .expect("one original admission publisher")
                .refused(error, None);
            return Err(error);
        }
        let permit = match self.cmd_tx.tx.try_reserve() {
            Ok(permit) => permit,
            Err(error) => {
                let error = match error {
                    mpsc::error::TrySendError::Full(()) => {
                        VoiceEndpointError::Protocol(VoiceError::Capacity)
                    }
                    mpsc::error::TrySendError::Closed(()) => VoiceEndpointError::Stopped,
                };
                preparing
                    .arguments
                    .publisher
                    .take()
                    .expect("one original admission publisher")
                    .refused(error, None);
                return Err(error);
            }
        };
        let work = VoiceAdmissionWork(Arc::new(InvocationState::new(
            preparing.payment.take().expect("one original RPC payment"),
        )));
        preparing.arguments.retirement = Some(InvocationRetirement(work.0.clone()));
        // Keep the untouched publisher in Arguments until enqueue. Unwind during
        // this custody-only callback is still proven never-entered, with Work.
        if let Err(error) = preparing
            .arguments
            .publisher
            .as_mut()
            .expect("one original admission publisher")
            .accepted(work.clone())
        {
            preparing
                .arguments
                .publisher
                .take()
                .expect("one original admission publisher")
                .refused(error, Some(work.clone()));
            return Err(error);
        }
        // Arguments implements Drop: move the entire ordered aggregate, never fields
        // out of it. Its unused-publisher retirement also covers Actor destruction.
        permit.send(Envelope {
            trace: self.cmd_tx.trace,
            command: AppCommand::VoiceEndpoint(VoiceEndpointCommand(super::Command::Admission(
                Command {
                    arguments: preparing.arguments,
                    payment: work.clone(),
                },
            ))),
        });
        Ok(work)
    }
}

/// Actual fixed Actor queue/preparation/callback wrapper graph, excluding the
/// concrete native Publisher (which must also fit the SAME4KiB before Box creation).
/// Root registration and HIGH capacity keep their own independent original payments.
pub const fn voice_admission_rpc_bytes() -> usize {
    std::mem::size_of::<Envelope>()
        + std::mem::size_of::<Command>()
        + std::mem::size_of::<Preparing>()
        + std::mem::size_of::<(usize, usize, InvocationState<AdmissionRpcPermit>)>()
        + 3 * std::mem::size_of::<VoiceAdmissionWork>()
        + std::mem::size_of::<Result<AdmissionPrepared, VoiceError>>()
        + 256 // fixed queue permit, argument borrows, alignment and callback return frames
}
