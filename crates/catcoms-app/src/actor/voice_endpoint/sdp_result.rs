//! Exact original Result RPC with eager, paid-last request construction. This shares
//! the existing Body/metadata channel, never a second result buffer or reply channel.
use super::sdp_body::{
    VoiceSdpBody, VoiceSdpCompletion, VoiceSdpPending, Work, VOICE_SDP_RPC_METADATA_BYTES,
};
use super::*;
use catcoms_sync::voice_v2::chain::sdp_handoff::Copy;
use catcoms_sync::voice_v2::sdp::{
    InitialSdpIntent, SdpOpenOutput, SdpOpenResult, SdpResultAdmission, SdpResultIdentity,
};

/// Trusted native final Result fence. The implementation must release its lexical
/// UI/route/row guards before returning, including refusal/unwind. A terminal retry may
/// bypass this hook; dropping an unused hook must not erase or downgrade that result.
/// Its concrete allocation is separately paid by the original native Task.
pub trait VoiceSdpResultPublisher: Send {
    fn publish(self: Box<Self>, step: SdpResultAdmission<'_>) -> Result<(), VoiceError>;
}

/// Original native input, claimed synchronously before the sender future exists.
pub enum VoiceSdpResultInput<'a> {
    Sealed {
        body: &'a VoiceSdpBody,
        tag: [u8; 32],
    },
    Opened {
        body: &'a VoiceSdpBody,
        tag: [u8; 32],
    },
    AuthenticationFailed(Copy),
}
/// Small terminal metadata only. An accepted Open observer is not fresh RTC permission.
pub enum VoiceSdpResult {
    Sealed(CryptoResultReceipt),
    Opened(SdpOpenResult),
}
impl VoiceSdpResult {
    pub fn receipt(&self) -> &CryptoResultReceipt {
        match self {
            Self::Sealed(receipt) => receipt,
            Self::Opened(result) => &result.receipt,
        }
    }
}
impl std::fmt::Debug for VoiceSdpResultInput<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceSdpResultInput { .. }")
    }
}
impl std::fmt::Debug for VoiceSdpResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceSdpResult { .. }")
    }
}

struct Arguments {
    grant: Arc<CallGrant>,
    intent: Arc<InitialSdpIntent>,
    action: Arc<CryptoAction>,
    publisher: Option<Box<dyn VoiceSdpResultPublisher>>,
    identity: SdpResultIdentity,
    seal: bool,
}
pub(super) struct Message {
    arguments: Arguments,
    reply: Reply<VoiceSdpCompletion<VoiceSdpResult>>,
}
pub(super) type Command = sdp_rpc::Command<Message, Work>;
type Request = Result<
    (
        Command,
        sdp_rpc::Pending<Result<VoiceSdpCompletion<VoiceSdpResult>, VoiceError>, Work>,
    ),
    VoiceEndpointError,
>;
// Native's affine publisher must drop BEFORE a rejected metadata Copy on any local
// error. Holding all intake in this ordered aggregate closes the unpolled-future gap.
struct Preparing<'a> {
    grant: Arc<CallGrant>,
    intent: Arc<InitialSdpIntent>,
    action: Arc<CryptoAction>,
    publisher: Box<dyn VoiceSdpResultPublisher>,
    input: Option<VoiceSdpResultInput<'a>>,
}
fn prepare(mut args: Preparing<'_>) -> Request {
    if !args.action.uses_sdp_intent(&args.intent) {
        return Err(VoiceEndpointError::Protocol(VoiceError::Unauthorized));
    }
    // Validate a metadata payment while it is still the LAST original argument. No
    // fallible operation follows its move into Work and the ordered Command.
    if let Some(VoiceSdpResultInput::AuthenticationFailed(copy)) = &args.input {
        super::sdp_ops::check_metadata(copy, &args.intent)?;
    }
    let (work, identity, seal) = match args.input.take().expect("one original input") {
        VoiceSdpResultInput::Sealed { body, tag } => (
            Work::Body(
                body.claim(&args.intent, catcoms_sync::voice_v2::MAX_SDP_PLAINTEXT + 16)
                    .map_err(VoiceEndpointError::Protocol)?,
            ),
            SdpResultIdentity::Bytes(tag),
            true,
        ),
        VoiceSdpResultInput::Opened { body, tag } => (
            Work::Body(
                body.claim(&args.intent, catcoms_sync::voice_v2::MAX_SDP_PLAINTEXT)
                    .map_err(VoiceEndpointError::Protocol)?,
            ),
            SdpResultIdentity::Bytes(tag),
            false,
        ),
        VoiceSdpResultInput::AuthenticationFailed(copy) => (
            Work::Metadata(copy),
            SdpResultIdentity::AuthenticationFailed,
            false,
        ),
    };
    let Preparing {
        grant,
        intent,
        action,
        publisher,
        input,
    } = args;
    debug_assert!(input.is_none());
    Ok(sdp_rpc::request(work, |reply| Message {
        arguments: Arguments {
            grant,
            intent,
            action,
            publisher: Some(publisher),
            identity,
            seal,
        },
        reply,
    }))
}
pub(super) fn apply<T: MeshTransport, R: CryptoRngCore>(
    mut command: Command,
    server: &mut Server<T, R>,
) {
    if command.message.reply.is_closed() {
        return;
    }
    // Keep the aggregate intact across Clock/native callbacks: publisher/arguments
    // always retire before Work, including an unwind in signing or final admission.
    let mut publisher = command.message.arguments.publisher.take();
    let args = &command.message.arguments;
    let result = if !command.work.uses_intent(&args.intent)
        || !args.action.uses_sdp_intent(&args.intent)
    {
        Err(VoiceError::Unauthorized)
    } else if args.seal {
        match args.identity {
            SdpResultIdentity::Bytes(tag) => command.work.bytes().and_then(|bytes| {
                server
                    .sync
                    .submit_voice_sdp_result_scoped(&args.grant, &args.action, bytes, tag, |step| {
                        publisher
                            .take()
                            .expect("one original native fence")
                            .publish(step)
                    })
                    .map(VoiceSdpResult::Sealed)
            }),
            SdpResultIdentity::AuthenticationFailed => Err(VoiceError::Unauthorized),
        }
    } else {
        let output = match args.identity {
            SdpResultIdentity::Bytes(tag) => {
                command.work.bytes().map(|bytes| SdpOpenOutput::Plaintext {
                    bytes,
                    claimed_tag: tag,
                })
            }
            SdpResultIdentity::AuthenticationFailed => Ok(SdpOpenOutput::AuthenticationFailed),
        };
        output
            .and_then(|output| {
                server.sync.submit_voice_sdp_open_result_scoped(
                    &args.grant,
                    &args.action,
                    output,
                    |step| {
                        publisher
                            .take()
                            .expect("one original native fence")
                            .publish(step)
                    },
                )
            })
            .map(VoiceSdpResult::Opened)
    };
    drop(publisher); // also the valid historical-success path which never lent a step
    let sdp_rpc::Command {
        message: Message { arguments, reply },
        work,
    } = command;
    drop(arguments);
    sdp_rpc::complete(reply, work, |work| {
        Ok(VoiceSdpCompletion::new(result, work))
    });
}

type Sending<'a> = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<VoiceSdpPending<VoiceSdpResult>, VoiceEndpointError>,
            > + Send
            + 'a,
    >,
>;
enum QueueState<'a> {
    Refused(Option<VoiceEndpointError>),
    Sending(Sending<'a>),
}
/// Original eager sender backing and observer, funded by the SAME Body/metadata RPC.
/// No new Copy/channel/task exists. A retained completed queue stays paid until dropped.
pub struct VoiceSdpResultQueue<'a> {
    state: QueueState<'a>,
    _work: Option<Arc<Work>>, // LAST, including backing destruction on cancellation
}
impl std::fmt::Debug for VoiceSdpResultQueue<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceSdpResultQueue { .. }")
    }
}
impl std::future::Future for VoiceSdpResultQueue<'_> {
    type Output = Result<VoiceSdpPending<VoiceSdpResult>, VoiceEndpointError>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match &mut self.get_mut().state {
            QueueState::Refused(error) => {
                std::task::Poll::Ready(Err(error.take().expect("queue polled after refusal")))
            }
            QueueState::Sending(future) => future.as_mut().poll(cx),
        }
    }
}
impl<'a> VoiceSdpResultQueue<'a> {
    fn new<F>(prepared: Request, send: impl FnOnce(Request) -> F) -> Self
    where
        F: std::future::Future<
                Output = Result<VoiceSdpPending<VoiceSdpResult>, VoiceEndpointError>,
            > + Send
            + 'a,
    {
        match prepared {
            Err(error) => Self {
                state: QueueState::Refused(Some(error)),
                _work: None,
            },
            Ok((command, pending)) => {
                let work = command.work.clone();
                let future = send(Ok((command, pending)));
                if super::sdp_body::voice_sdp_result_rpc_bytes(
                    std::mem::size_of::<F>() + std::mem::size_of::<Self>(),
                ) > VOICE_SDP_RPC_METADATA_BYTES
                {
                    drop(future);
                    return Self {
                        state: QueueState::Refused(Some(VoiceEndpointError::Protocol(
                            VoiceError::Capacity,
                        ))),
                        _work: Some(work),
                    };
                }
                Self {
                    state: QueueState::Sending(Box::pin(future)),
                    _work: Some(work),
                }
            }
        }
    }
}
impl ServerActor {
    /// Eagerly claim the original buffer and put the affine native fence into the
    /// ordered Command BEFORE returning a future. An unpolled/cancelled send cannot
    /// release its last payment ahead of publisher/argument destruction.
    pub fn queue_voice_sdp_result_scoped(
        &self,
        grant: Arc<CallGrant>,
        intent: Arc<InitialSdpIntent>,
        action: Arc<CryptoAction>,
        input: VoiceSdpResultInput<'_>,
        publisher: Box<dyn VoiceSdpResultPublisher>,
    ) -> VoiceSdpResultQueue<'_> {
        let request = prepare(Preparing {
            grant,
            intent,
            action,
            publisher,
            input: Some(input),
        });
        VoiceSdpResultQueue::new(request, |request| self.send_sdp_result(request))
    }
    async fn send_sdp_result(
        &self,
        prepared: Request,
    ) -> Result<VoiceSdpPending<VoiceSdpResult>, VoiceEndpointError> {
        let (command, pending) = prepared?;
        // Await only a mailbox position. Construct the ONE envelope after capacity
        // exists, so the paid sender backing contains no nested send future holding
        // another full AppCommand beside the original ordered request.
        let permit = self
            .cmd_tx
            .tx
            .reserve()
            .await
            .map_err(|_| VoiceEndpointError::Stopped)?;
        permit.send(Envelope {
            trace: self.cmd_tx.trace,
            command: AppCommand::VoiceEndpoint(VoiceEndpointCommand(super::Command::SdpResult(
                command,
            ))),
        });
        Ok(VoiceSdpPending(pending))
    }
}

/// Common RPC graph adds these concrete eager constructor arguments and the ONE
/// moved request pair, not just the thin public future. No Body backing is duplicated.
struct Construction {
    _preparing: Preparing<'static>,
    _request: Request,
    _work: Work,
    _anchor: Arc<Work>,
    _message: Message,
    _reply: Reply<VoiceSdpCompletion<VoiceSdpResult>>,
    _receiver: oneshot::Receiver<Result<VoiceSdpCompletion<VoiceSdpResult>, VoiceError>>,
}
pub(super) const CONSTRUCTION_BYTES: usize = std::mem::size_of::<Construction>();

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scoped_result_eager_refusal_never_constructs_or_allocates_sender_backing() {
        let queue = VoiceSdpResultQueue::new(
            Err(VoiceEndpointError::Protocol(VoiceError::Capacity)),
            |_| -> std::future::Ready<Result<VoiceSdpPending<VoiceSdpResult>, VoiceEndpointError>> {
                panic!("invalid input must not construct a sender")
            },
        );
        assert!(matches!(queue.state, QueueState::Refused(_)));
        assert!(queue._work.is_none());
    }
    #[test]
    fn scoped_result_actual_sender_observer_and_collection_fit_original_rpc_contribution() {
        fn sender<F>(_: fn(&'static ServerActor, Request) -> F) -> usize {
            std::mem::size_of::<F>()
        }
        fn collect<F>(_: fn(VoiceSdpPending<VoiceSdpResult>) -> F) -> usize {
            std::mem::size_of::<F>()
        }
        let future = sender(ServerActor::send_sdp_result)
            + std::mem::size_of::<VoiceSdpResultQueue<'static>>();
        let total = super::super::sdp_body::voice_sdp_result_rpc_bytes(future);
        eprintln!("actual Result sender/observer graph {total}/{VOICE_SDP_RPC_METADATA_BYTES}; backing {future}; eager {CONSTRUCTION_BYTES}");
        assert!(total <= VOICE_SDP_RPC_METADATA_BYTES);
        assert!(
            super::super::sdp_body::voice_sdp_result_rpc_bytes(collect(VoiceSdpPending::complete))
                <= VOICE_SDP_RPC_METADATA_BYTES
        );
    }
}
