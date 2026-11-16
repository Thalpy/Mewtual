//! Paid first-media publication on the actual sole Actor. Original SDP Phase custody pays
//! this RPC: first admission still requires that phase/Ready. The detached media budget is
//! not silently treated as4KiB (its reservation header leaves less). No live App activation.
use super::sdp_input::{InvocationRetirement, InvocationState, Paid};
use super::*;
use catcoms_sync::voice_v2::chain::sdp_handoff::Copy;
use catcoms_sync::voice_v2::sdp::{InitialSdpIntent, MediaAdmission, SdpMedia};

pub const VOICE_INITIAL_MEDIA_RPC_BYTES: usize = 4096;

/// Trusted native-only consuming hook. Its concrete publisher, native output/oneshot and
/// callback graph need SEPARATE prepayment and layout measurement before Box allocation.
/// Native enters UI/Coordinator/route/original-row gates around `step.commit`, requires
/// canonical original Seal/Open localDrained plus live original key/SDK, then installs the
/// already prepared SAME media claim without callbacks/fallible allocation. No SDK call,
/// Clock or async continuation belongs under those gates. Exact native lost-reply history
/// recovery precedes this first-admission RPC; it never queues another first transition.
pub trait VoiceInitialMediaPublisher: Send {
    fn publish(self: Box<Self>, step: MediaAdmission<'_>, work: VoiceInitialMediaWork);
    fn refused(self: Box<Self>, error: VoiceEndpointError);
}

/// Original actual queue/callback custody, not a media grant or SDK drainage statement.
/// Caller/output and the running sender retain the SAME payment through callback return.
#[derive(Clone)]
pub struct VoiceInitialMediaWork(Arc<InvocationState<Copy>>);
impl VoiceInitialMediaWork {
    /// Join actual Actor application/callback return (or destruction before application).
    /// Does not prove publication absence, current media authority or native/browser IO
    /// collection. Native history may only sample its clocks AFTER this lifecycle joins.
    pub async fn wait_retired(&self) {
        self.0.wait_retired().await;
    }
}
impl std::fmt::Debug for VoiceInitialMediaWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceInitialMediaWork { .. }")
    }
}
pub(super) struct Arguments {
    grant: Arc<CallGrant>,
    intent: Arc<InitialSdpIntent>,
    media: Arc<SdpMedia>,
    seal: Arc<CryptoAction>,
    open: Arc<CryptoAction>,
    publisher: Option<Box<dyn VoiceInitialMediaPublisher>>,
    // LAST argument: all actual original references drop before lifecycle retirement,
    // with the ordered Paid aggregate still retaining its separate RPC contribution.
    retirement: Option<InvocationRetirement<Copy>>,
}
pub(super) type Command = Paid<Arguments, VoiceInitialMediaWork>;
type Preparing = Paid<Arguments, Option<Copy>>;

pub(super) fn apply<T: MeshTransport, R: CryptoRngCore>(
    mut command: Command,
    server: &mut Server<T, R>,
) {
    // Do not decompose the ordered aggregate: callbacks can reenter or unwind, and an
    // immediate receiver must never refund the still-returning sender's original fee.
    let mut publisher = command.arguments.publisher.take();
    let result = server.sync.with_voice_initial_media_admission(
        &command.arguments.grant,
        &command.arguments.intent,
        &command.arguments.media,
        &command.arguments.seal,
        &command.arguments.open,
        |step| {
            publisher
                .take()
                .expect("one original media publisher")
                .publish(step, command.payment.clone());
            Ok(())
        },
    );
    if let Some(publisher) = publisher {
        publisher.refused(VoiceEndpointError::Protocol(
            result.err().unwrap_or(VoiceError::Unauthorized),
        ));
    }
}

impl ServerActor {
    /// Queue without parking an allocated envelope behind mailbox capacity. This payment
    /// must come from the SAME original native phase and own no UI/publication keepalive.
    /// Canonical SDP-row collection must precede native first admission; an old row's
    /// bound IO contribution is not evidence that this distinct operation is collected.
    #[allow(clippy::too_many_arguments)] // Independently authenticated original owners/payment/hook.
    pub fn try_queue_voice_initial_media(
        &self,
        grant: Arc<CallGrant>,
        intent: Arc<InitialSdpIntent>,
        media: Arc<SdpMedia>,
        seal: Arc<CryptoAction>,
        open: Arc<CryptoAction>,
        payment: Copy,
        publisher: Box<dyn VoiceInitialMediaPublisher>,
    ) -> Result<VoiceInitialMediaWork, VoiceEndpointError> {
        let mut preparing = Preparing {
            arguments: Arguments {
                grant,
                intent,
                media,
                seal,
                open,
                publisher: Some(publisher),
                retirement: None,
            },
            payment: Some(payment),
        };
        let payment = preparing
            .payment
            .as_ref()
            .expect("original media RPC payment");
        let checked = if !payment.uses_intent(&preparing.arguments.intent)
            || payment.has_keepalive()
            || !preparing
                .arguments
                .media
                .uses_intent(&preparing.arguments.intent)
            || !preparing
                .arguments
                .seal
                .uses_sdp_intent(&preparing.arguments.intent)
            || !preparing
                .arguments
                .open
                .uses_sdp_intent(&preparing.arguments.intent)
        {
            Err(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
        } else if !payment.covers_work(VOICE_INITIAL_MEDIA_RPC_BYTES) {
            Err(VoiceEndpointError::Protocol(VoiceError::Capacity))
        } else {
            Ok(())
        };
        if let Err(error) = checked {
            preparing
                .arguments
                .publisher
                .take()
                .expect("one original media publisher")
                .refused(error);
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
                    .expect("one original media publisher")
                    .refused(error);
                return Err(error);
            }
        };
        let work = VoiceInitialMediaWork(Arc::new(InvocationState::new(
            preparing.payment.take().expect("one original RPC payment"),
        )));
        preparing.arguments.retirement = Some(InvocationRetirement(work.0.clone()));
        permit.send(Envelope {
            trace: self.cmd_tx.trace,
            command: AppCommand::VoiceEndpoint(VoiceEndpointCommand(super::Command::InitialMedia(
                Command {
                    arguments: preparing.arguments,
                    payment: work.clone(),
                },
            ))),
        });
        Ok(work)
    }
}

/// Actual synchronous Actor queue/callback graph, apart from the independently paid native
/// concrete publisher/oneshot/output. All arguments, lifecycle owners and Root origin overlap.
pub const fn voice_initial_media_rpc_bytes() -> usize {
    std::mem::size_of::<Envelope>()
        + std::mem::size_of::<Command>()
        + std::mem::size_of::<Preparing>()
        + std::mem::size_of::<(usize, usize, InvocationState<Copy>)>()
        + 3 * std::mem::size_of::<VoiceInitialMediaWork>()
        + catcoms_sync::voice_v2::sdp::initial_media_admission_bytes()
        + 256
}
const _: () = assert!(voice_initial_media_rpc_bytes() <= VOICE_INITIAL_MEDIA_RPC_BYTES);
