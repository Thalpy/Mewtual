//! Separate paid engine admission on the actual Actor. The original Exchange owns
//! crypto credit; this native-phase contribution pays only the queue/callback graph.
//! UI is lexical through commit, never retained by the output across Root cleanup.
use super::sdp_input::{InvocationRetirement, InvocationState, Paid};
use super::*;
use catcoms_sync::voice_v2::chain::sdp_handoff::Copy;
use catcoms_sync::voice_v2::sdp::{InitialSdpIntent, SdpEngineAdmission};

pub const VOICE_SDP_ENGINE_RPC_BYTES: usize = 4096;

/// Trusted native-only publication hook; never constructed from an IPC descriptor. Its
/// concrete implementation must prepay and measure its WHOLE allocation, response backing,
/// oneshot, native engine/publication/output graph and future BEFORE constructing this Box.
/// A trait-reported size is not proof. This module pays its separate actual actor envelope.
///
/// Both methods consume the hook synchronously. publish may enter the prescribed native
/// UI/Coordinator/Route/row fence and commit its scoped Root step; it may not await or call
/// Clock/Root. Native holds UI only through commit and releases it BEFORE returning this hook.
/// Root may destroy an unused candidate after return. Retain physical/Task and this SAME
/// Work through output collection; a permit does not authorize SDK reexecution. Refusal cannot create a permit or retry an action factory.
pub trait VoiceSdpEnginePublisher: Send {
    fn publish(self: Box<Self>, step: SdpEngineAdmission<'_>, work: VoiceSdpEngineWork);
    fn refused(self: Box<Self>, error: VoiceEndpointError);
}

/// Original actor-work custody, not engine or publication authority. Both the
/// queue/caller and actual returned output retain it; a synchronous receiver cannot refund
/// the sender while its consuming native callback is still returning.
#[derive(Clone)]
pub struct VoiceSdpEngineWork(Arc<InvocationState<Copy>>);
impl VoiceSdpEngineWork {
    /// Wait for the actual scoped Root call AND consuming native callback to return,
    /// or for a never-applied command to be destroyed. This proves neither absence of
    /// publication, input/output collection nor current authority. History must wait here
    /// before invoking callback-bearing canonical row recovery outside native guards.
    pub async fn wait_retired(&self) {
        self.0.wait_retired().await;
    }
}
impl std::fmt::Debug for VoiceSdpEngineWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No original Copy or native publication owner is exposed by this observation.
        f.write_str("VoiceSdpEngineWork { .. }")
    }
}
pub(super) struct Arguments {
    grant: Arc<CallGrant>,
    intent: Arc<InitialSdpIntent>,
    action: Arc<CryptoAction>,
    bytes: usize,
    publisher: Option<Box<dyn VoiceSdpEnginePublisher>>,
    // Last argument: normal return/unwind drops every original argument before the
    // lifecycle signal, while Paid still retains actual RPC payment below this graph.
    retirement: Option<InvocationRetirement<Copy>>,
}
pub(super) type Command = Paid<Arguments, VoiceSdpEngineWork>;
type Preparing = Paid<Arguments, Option<Copy>>;
pub(super) fn apply<T: MeshTransport, R: CryptoRngCore>(
    mut command: Command,
    server: &mut Server<T, R>,
) {
    // Keep the ordered aggregate intact across Clock and native callbacks. Turning
    // its fields into locals would drop Work before arguments during an unwind.
    let mut publisher = command.arguments.publisher.take();
    let result = server.sync.with_voice_sdp_engine_admission(
        &command.arguments.grant,
        &command.arguments.intent,
        &command.arguments.action,
        command.arguments.bytes,
        |step| {
            publisher
                .take()
                .expect("one original engine publisher")
                .publish(step, command.payment.clone());
            Ok(())
        },
    );
    if let Some(publisher) = publisher {
        // Root refused before lending its callback. Keep the actual Work through this
        // native refusal and synchronous send, just as through successful publication.
        publisher.refused(VoiceEndpointError::Protocol(
            result.expect_err("Root refused before callback"),
        ));
    }
}
impl ServerActor {
    /// Admit one already-paid hook without parking a large envelope behind mailbox
    /// capacity. Full is a proven local refusal and calls this same hook's refusal path;
    /// closed is Stopped. No UI guard is acquired before the actor's Clock observations.
    /// On success the caller keeps returned Work through its actual receiver/output.
    pub fn try_queue_voice_sdp_engine(
        &self,
        grant: Arc<CallGrant>,
        intent: Arc<InitialSdpIntent>,
        action: Arc<CryptoAction>,
        bytes: usize,
        payment: Copy,
        publisher: Box<dyn VoiceSdpEnginePublisher>,
    ) -> Result<VoiceSdpEngineWork, VoiceEndpointError> {
        let mut preparing = Preparing {
            arguments: Arguments {
                grant,
                intent,
                action,
                bytes,
                publisher: Some(publisher),
                retirement: None,
            },
            payment: Some(payment),
        };
        let payment = preparing.payment.as_ref().expect("original RPC payment");
        let checked = if !payment.uses_intent(&preparing.arguments.intent)
            || payment.has_keepalive()
            || !preparing
                .arguments
                .action
                .uses_sdp_intent(&preparing.arguments.intent)
        {
            Err(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
        } else if bytes != preparing.arguments.action.engine_bytes() {
            Err(VoiceEndpointError::Protocol(VoiceError::RetryMismatch))
        } else if !payment.covers_work(VOICE_SDP_ENGINE_RPC_BYTES) {
            Err(VoiceEndpointError::Protocol(VoiceError::Capacity))
        } else {
            Ok(())
        };
        if let Err(error) = checked {
            preparing
                .arguments
                .publisher
                .take()
                .expect("one original engine publisher")
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
                    .expect("one original engine publisher")
                    .refused(error);
                return Err(error);
            }
        };
        let work = VoiceSdpEngineWork(Arc::new(InvocationState::new(
            preparing.payment.take().expect("one original RPC payment"),
        )));
        preparing.arguments.retirement = Some(InvocationRetirement(work.0.clone()));
        permit.send(Envelope {
            trace: self.cmd_tx.trace,
            command: AppCommand::VoiceEndpoint(VoiceEndpointCommand(super::Command::SdpEngine(
                Command {
                    arguments: preparing.arguments,
                    payment: work.clone(),
                },
            ))),
        });
        Ok(work)
    }
}

/// Actual fixed App queue/callback graph. Native's concrete publisher/oneshot/future and
/// response bytes have SEPARATE original payments; no trait-reported subtraction occurs.
pub const fn voice_sdp_engine_rpc_bytes() -> usize {
    std::mem::size_of::<Envelope>()
        + std::mem::size_of::<Command>()
        + std::mem::size_of::<Preparing>()
        + std::mem::size_of::<(usize, usize, InvocationState<Copy>)>()
        + 3 * std::mem::size_of::<VoiceSdpEngineWork>()
        + catcoms_sync::voice_v2::sdp::sdp_engine_admission_bytes()
        + 256 // bounded queue permit, closure, result and argument bookkeeping
}
const _: () = assert!(voice_sdp_engine_rpc_bytes() <= VOICE_SDP_ENGINE_RPC_BYTES);
