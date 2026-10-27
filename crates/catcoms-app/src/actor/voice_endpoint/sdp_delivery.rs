//! Separate paid delivery admission on the actual Actor. This native-phase contribution
//! pays only the fixed queue/callback graph; original Action/delivery history has its own fee.
//! UI is lexical through commit, never retained by the output across Root cleanup.
use super::sdp_input::{InvocationRetirement, InvocationState, Paid};
use super::*;
use catcoms_sync::voice_v2::chain::sdp_handoff::Copy;
use catcoms_sync::voice_v2::sdp::{InitialSdpIntent, SdpDelivery, SdpDeliveryAdmission};

pub const VOICE_SDP_DELIVERY_RPC_BYTES: usize = 4096;

/// Trusted native-only publication hook; never constructed from an IPC descriptor. Its
/// concrete implementation must prepay and measure its WHOLE allocation, response backing,
/// oneshot, native delivery/publication/output graph and future BEFORE constructing this Box.
/// A trait-reported size is not proof. This module pays its separate actual actor envelope.
///
/// All methods consume the hook synchronously. publish may enter the prescribed native
/// UI/Coordinator/Route/row fence and commit its scoped Root step; it may not await or call
/// Clock/Root. Native holds UI only through commit and releases it BEFORE returning this hook.
/// Root finishes its borrowed origin frame after return. Retain physical/Task and this
/// SAME Work through output collection; history never authorizes another RTC execution.
pub trait VoiceSdpDeliveryPublisher: Send {
    fn publish(self: Box<Self>, step: SdpDeliveryAdmission<'_>, work: VoiceSdpDeliveryWork);
    /// Recover only exact already-Applied metadata; do not run native admission here.
    fn historical(self: Box<Self>, work: VoiceSdpDeliveryWork);
    fn refused(self: Box<Self>, error: VoiceEndpointError);
}

/// Original actor-work custody, not engine or publication authority. Both the
/// queue/caller and actual returned output retain it; a synchronous receiver cannot refund
/// the sender while its consuming native callback is still returning.
#[derive(Clone)]
pub struct VoiceSdpDeliveryWork(Arc<InvocationState<Copy>>);
impl VoiceSdpDeliveryWork {
    /// Wait for the actual scoped Root call AND consuming native callback to return,
    /// or for a never-applied command to be destroyed. This proves neither absence of
    /// publication, input/output collection nor current authority. History must wait here
    /// before invoking callback-bearing canonical row recovery outside native guards.
    pub async fn wait_retired(&self) {
        self.0.wait_retired().await;
    }
}
impl std::fmt::Debug for VoiceSdpDeliveryWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No original Copy or native publication owner is exposed by this observation.
        f.write_str("VoiceSdpDeliveryWork { .. }")
    }
}
pub(super) struct Arguments {
    grant: Arc<CallGrant>,
    intent: Arc<InitialSdpIntent>,
    action: Arc<CryptoAction>,
    delivery: Arc<SdpDelivery>,
    applied: bool,
    publisher: Option<Box<dyn VoiceSdpDeliveryPublisher>>,
    // Last argument: normal return/unwind drops every original argument before the
    // lifecycle signal, while Paid still retains actual RPC payment below this graph.
    retirement: Option<InvocationRetirement<Copy>>,
}
pub(super) type Command = Paid<Arguments, VoiceSdpDeliveryWork>;
type Preparing = Paid<Arguments, Option<Copy>>;
pub(super) fn apply<T: MeshTransport, R: CryptoRngCore>(
    mut command: Command,
    server: &mut Server<T, R>,
) {
    // Keep the ordered aggregate intact across Clock and native callbacks. Turning
    // its fields into locals would drop Work before arguments during an unwind.
    let mut publisher = command.arguments.publisher.take();
    let result = server.sync.with_voice_sdp_delivery_admission(
        &command.arguments.grant,
        &command.arguments.action,
        &command.arguments.delivery,
        command.arguments.applied,
        |step| {
            publisher
                .take()
                .expect("one original delivery publisher")
                .publish(step, command.payment.clone());
            Ok(())
        },
    );
    if let Some(publisher) = publisher {
        // Exact already-Applied completion can bypass an unused hook successfully.
        // It is immutable history, never another current RTC admission or SDK permission.
        match result {
            Ok(()) => publisher.historical(command.payment.clone()),
            Err(error) => publisher.refused(VoiceEndpointError::Protocol(error)),
        }
    }
}
impl ServerActor {
    /// Admit one already-paid hook without parking a large envelope behind mailbox
    /// capacity. Full is a proven local refusal and calls this same hook's refusal path;
    /// closed is Stopped. No UI guard is acquired before the actor's Clock observations.
    /// On success the caller keeps returned Work through its actual receiver/output.
    #[allow(clippy::too_many_arguments)] // Independent original effect, owner, funding and publication provenance.
    pub fn try_queue_voice_sdp_delivery(
        &self,
        grant: Arc<CallGrant>,
        intent: Arc<InitialSdpIntent>,
        action: Arc<CryptoAction>,
        delivery: Arc<SdpDelivery>,
        applied: bool,
        payment: Copy,
        publisher: Box<dyn VoiceSdpDeliveryPublisher>,
    ) -> Result<VoiceSdpDeliveryWork, VoiceEndpointError> {
        let mut preparing = Preparing {
            arguments: Arguments {
                grant,
                intent,
                action,
                delivery,
                applied,
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
        } else if !payment.covers_work(VOICE_SDP_DELIVERY_RPC_BYTES) {
            Err(VoiceEndpointError::Protocol(VoiceError::Capacity))
        } else {
            Ok(())
        };
        if let Err(error) = checked {
            preparing
                .arguments
                .publisher
                .take()
                .expect("one original delivery publisher")
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
                    .expect("one original delivery publisher")
                    .refused(error);
                return Err(error);
            }
        };
        let work = VoiceSdpDeliveryWork(Arc::new(InvocationState::new(
            preparing.payment.take().expect("one original RPC payment"),
        )));
        preparing.arguments.retirement = Some(InvocationRetirement(work.0.clone()));
        permit.send(Envelope {
            trace: self.cmd_tx.trace,
            command: AppCommand::VoiceEndpoint(VoiceEndpointCommand(super::Command::SdpDelivery(
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
pub const fn voice_sdp_delivery_rpc_bytes() -> usize {
    std::mem::size_of::<Envelope>()
        + std::mem::size_of::<Command>()
        + std::mem::size_of::<Preparing>()
        + std::mem::size_of::<(usize, usize, InvocationState<Copy>)>()
        + 3 * std::mem::size_of::<VoiceSdpDeliveryWork>()
        + catcoms_sync::voice_v2::sdp::sdp_delivery_admission_bytes()
        + 256 // bounded queue permit, closure, result and argument bookkeeping
}
const _: () = assert!(voice_sdp_delivery_rpc_bytes() <= VOICE_SDP_DELIVERY_RPC_BYTES);
