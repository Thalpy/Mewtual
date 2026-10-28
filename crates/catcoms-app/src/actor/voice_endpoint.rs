//! Trusted native voice controls and opaque prepaid result owners through the bounded actor
//! mailbox. Root owns protocol state and authorizes every first effect in its serialized turn.
//! Plaintext stays with its actual native owner; no Tauri command, byte IPC or live dispatch.
use super::*;
use catcoms_sync::voice_v2::crypto_actions::{
    CryptoAction, CryptoActionSnapshot, CryptoActionSnapshotPermit, CryptoResultReceipt,
};
use catcoms_sync::voice_v2::presence::{
    FreshPeer, PairReceipt, PairSelector, PeerSnapshot, PeerSnapshotPermit, VoicePairConfirmation,
};
use catcoms_sync::voice_v2::{CallGrant, CallLease, Nonce, VoiceError};
use catcoms_sync::VoiceWatchGrant;

/// Keep temporary protocol capacity distinct from actor loss without parsing diagnostic text.
/// The native adapter still owns exact retry/copy/settlement custody and caller authorization.
/// Stopped is uncertain: an admitted request may have completed before losing its reply. Retry
/// only the same original request/owners; never infer permission to allocate another candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceEndpointError {
    Stopped,
    Protocol(VoiceError),
}
impl std::fmt::Display for VoiceEndpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped => f.write_str("voice server stopped"),
            Self::Protocol(error) => std::fmt::Display::fmt(error, f),
        }
    }
}
impl std::error::Error for VoiceEndpointError {}

type Reply<T> = oneshot::Sender<Result<T, VoiceError>>;

mod admission;
mod body;
pub use admission::{voice_admission_rpc_bytes, VoiceAdmissionPublisher, VoiceAdmissionWork};
mod context;
pub use context::{voice_context_queue_bytes, VoiceContextPending};
mod sdp_body;
mod sdp_delivery;
mod sdp_engine;
mod sdp_input;
mod sdp_media;
mod sdp_ops;
mod sdp_result;
pub use body::{
    VoiceConfirmationBody, VoiceConfirmationCompletion, VOICE_CONFIRMATION_BODY_METADATA_BYTES,
};
pub use sdp_body::{
    voice_sdp_rpc_bytes, VoiceSdpBody, VoiceSdpBodyRefusal, VoiceSdpCompletion, VoiceSdpPending,
    VOICE_SDP_RPC_FIXED_BYTES, VOICE_SDP_RPC_METADATA_BYTES,
};
pub use sdp_delivery::{
    voice_sdp_delivery_rpc_bytes, VoiceSdpDeliveryPublisher, VoiceSdpDeliveryWork,
    VOICE_SDP_DELIVERY_RPC_BYTES,
};
pub use sdp_engine::{
    voice_sdp_engine_rpc_bytes, VoiceSdpEnginePublisher, VoiceSdpEngineWork,
    VOICE_SDP_ENGINE_RPC_BYTES,
};
pub use sdp_input::{
    voice_sdp_input_rpc_bytes, VoiceSdpInputPublisher, VoiceSdpInputWork, VOICE_SDP_INPUT_RPC_BYTES,
};
pub use sdp_media::{
    voice_initial_media_rpc_bytes, VoiceInitialMediaPublisher, VoiceInitialMediaWork,
    VOICE_INITIAL_MEDIA_RPC_BYTES,
};
pub use sdp_ops::{VoiceSdpPublication, VoiceSdpRecordedInput, VoiceSdpRecordedQueue};
pub use sdp_result::{
    VoiceSdpResult, VoiceSdpResultInput, VoiceSdpResultPublisher, VoiceSdpResultQueue,
};

/// Actual queued/result ownership for one initial SDP-intent RPC. A lost observer or
/// closed reply does not refund its phase contribution before Root and the reply retire.
/// It never enters the retained Pair/intent, so it cannot create a Phase custody cycle.
pub struct InitialSdpCompletion {
    result: Result<Arc<catcoms_sync::voice_v2::sdp::InitialSdpIntent>, VoiceError>,
    // LAST: the whole command/result graph must die before actual phase drainage.
    _work: Arc<InitialSdpWork>,
}
impl std::fmt::Debug for InitialSdpCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Neither protocol identities nor a historical result are diagnostic authority.
        f.write_str("InitialSdpCompletion { .. }")
    }
}
impl InitialSdpCompletion {
    pub fn result(
        &self,
    ) -> Result<&Arc<catcoms_sync::voice_v2::sdp::InitialSdpIntent>, VoiceError> {
        self.result.as_ref().map_err(|error| *error)
    }
}

/// Observation of one already queued original request. Dropping this receiver does not
/// cancel/refund the Command's real Copy. It may still publish and must be recoverable.
pub struct InitialSdpPending(
    sdp_rpc::Pending<Result<InitialSdpCompletion, VoiceError>, InitialSdpWork>,
);
impl std::fmt::Debug for InitialSdpPending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InitialSdpPending { .. }")
    }
}
impl InitialSdpPending {
    pub async fn complete(self) -> Result<InitialSdpCompletion, VoiceEndpointError> {
        self.0
            .receive()
            .await
            .map_err(|_| VoiceEndpointError::Stopped)?
            .map_err(VoiceEndpointError::Protocol)
    }
}

// Keep both channel sides and completions on one affine contribution. The generic
// wrapper's destructor tests exercise the exact production field and cancellation order.
mod sdp_rpc;
type InitialSdpWork = catcoms_sync::voice_v2::chain::sdp_handoff::Copy;
struct InitialSdpMessage {
    grant: Arc<CallGrant>,
    handshake: Nonce,
    key: catcoms_sync::voice_v2::chain::EstablishedKeyCustody,
    effect: catcoms_sync::voice_v2::chain::sdp_handoff::EffectObserver,
    generation: u64,
    reply: Reply<InitialSdpCompletion>,
}

/// Opaque envelope required by the public AppCommand type. Its private variants have no public
/// constructors or serialization; trusted native callers use the typed ServerActor methods.
pub struct VoiceEndpointCommand(Command);
impl std::fmt::Debug for VoiceEndpointCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceEndpointCommand { .. }")
    }
}
impl VoiceEndpointCommand {
    pub(super) fn apply<T: MeshTransport, R: CryptoRngCore>(self, server: &mut Server<T, R>) {
        self.0.apply(server);
    }
}

enum Command {
    Admission(admission::Command),
    Sdp(sdp_ops::Command),
    SdpInput(sdp_input::Command),
    SdpEngine(sdp_engine::Command),
    SdpResult(sdp_result::Command),
    SdpDelivery(sdp_delivery::Command),
    InitialMedia(sdp_media::Command),
    Stop {
        grant: Arc<CallGrant>,
        reply: Reply<()>,
    },
    Routing {
        routing: catcoms_sync::VoiceRouting,
        reply: Reply<()>,
    },
    ParticipantPeers {
        grant: Arc<CallGrant>,
        reply: Reply<Vec<FreshPeer>>,
    },
    ParticipantPeerSnapshot {
        grant: Arc<CallGrant>,
        permit: PeerSnapshotPermit,
        reply: Reply<PeerSnapshot>,
    },
    Start {
        grant: Arc<CallGrant>,
        join: Nonce,
        public_key: [u8; 65],
        reply: Reply<()>,
    },
    Watch {
        room: u128,
        parent: CallLease,
        reply: Reply<Arc<VoiceWatchGrant>>,
    },
    Peers {
        grant: Arc<VoiceWatchGrant>,
        reply: Reply<Vec<FreshPeer>>,
    },
    Changes {
        reply: Reply<tokio::sync::watch::Receiver<u64>>,
    },
    ConfirmationActions {
        grant: Arc<CallGrant>,
        permit: CryptoActionSnapshotPermit,
        reply: Reply<CryptoActionSnapshot>,
    },
    Pair {
        grant: Arc<CallGrant>,
        selector: PairSelector,
        reply: Reply<PairReceipt>,
    },
    PairConfirmation {
        grant: Arc<CallGrant>,
        selector: PairSelector,
        handshake: Nonce,
        reply: Reply<VoicePairConfirmation>,
    },
    HighReady {
        grant: Arc<CallGrant>,
        handshake: Nonce,
        // Fixed mailbox ownership: a native caller cannot retain an arbitrary helper Vec.
        helpers: [Option<DeviceId>; 2],
        reply: Reply<bool>,
    },
    InitialSdpIntent(sdp_rpc::Command<InitialSdpMessage, InitialSdpWork>),
    HighReadyConnected {
        grant: Arc<CallGrant>,
        handshake: Nonce,
        reply: Reply<bool>,
    },
    QueueReady {
        grant: Arc<CallGrant>,
        handshake: Nonce,
        reply: Reply<bool>,
    },
    PrepayLowSdp {
        grant: Arc<CallGrant>,
        handshake: Nonce,
        reply: Reply<()>,
    },
    LowSdp {
        grant: Arc<CallGrant>,
        handshake: Nonce,
        reply: Reply<bool>,
    },
    Derive {
        grant: Arc<CallGrant>,
        handshake: Nonce,
        reply: Reply<Arc<CryptoAction>>,
    },
    Derived {
        grant: Arc<CallGrant>,
        action: Arc<CryptoAction>,
        tag: [u8; 32],
        reply: Reply<CryptoResultReceipt>,
    },
    ConfirmationResult {
        grant: Arc<CallGrant>,
        action: Arc<CryptoAction>,
        body: body::Flight,
        tag: [u8; 32],
        open: bool,
        reply: Reply<VoiceConfirmationCompletion>,
    },
    StartConfirmation {
        grant: Arc<CallGrant>,
        handshake: Nonce,
        predecessor: Option<Nonce>,
        reply: Reply<Arc<CryptoAction>>,
    },
    RespondConfirmation {
        grant: Arc<CallGrant>,
        handshake: Nonce,
        trigger: Arc<CryptoAction>,
        reply: Reply<Arc<CryptoAction>>,
    },
}

impl Command {
    pub(super) fn apply<T: MeshTransport, R: CryptoRngCore>(self, server: &mut Server<T, R>) {
        // Losing a reply is not permission to repeat a first effect. Root factories recover
        // the actual action; terminal metadata survives for exact result recovery. A lost Watch
        // reply instead drops its affine grant and closes precisely that abandoned observer.
        match self {
            Self::Admission(command) => admission::apply(command, server),
            Self::Sdp(command) => sdp_ops::apply(command, server),
            Self::SdpInput(command) => sdp_input::apply(command, server),
            Self::SdpEngine(command) => sdp_engine::apply(command, server),
            Self::SdpResult(command) => sdp_result::apply(command, server),
            Self::SdpDelivery(command) => sdp_delivery::apply(command, server),
            Self::InitialMedia(command) => sdp_media::apply(command, server),
            Self::Stop { grant, reply } => {
                // An admitted cleanup must run even if its reply is lost. Unlike a factory,
                // this only retires the SAME original allocation and cannot create authority.
                server.sync.stop_voice_endpoint(&grant);
                let _ = reply.send(Ok(()));
            }
            Self::Routing { routing, reply } => {
                if !reply.is_closed() {
                    server.sync.set_voice_routing(routing);
                    let _ = reply.send(Ok(()));
                }
            }
            Self::ParticipantPeers { grant, reply } => {
                if !reply.is_closed() {
                    let _ = reply.send(server.sync.voice_participant_peers(&grant));
                }
            }
            Self::ParticipantPeerSnapshot {
                grant,
                permit,
                reply,
            } => {
                if !reply.is_closed() {
                    let _ = reply.send(Ok(server
                        .sync
                        .voice_participant_peer_snapshot(&grant, permit)));
                }
            }
            Self::Start {
                grant,
                join,
                public_key,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                let _ = reply.send(server.sync.start_voice_endpoint(grant, join, public_key));
            }
            Self::Watch {
                room,
                parent,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                let _ = reply.send(server.sync.start_voice_watch(room, parent));
            }
            Self::Peers { grant, reply } => {
                if reply.is_closed() {
                    return;
                }
                let _ = reply.send(server.sync.voice_watch_peers(&grant));
            }
            Self::Changes { reply } => {
                if reply.is_closed() {
                    return;
                }
                let _ = reply.send(server.sync.voice_endpoint_changes());
            }
            Self::ConfirmationActions {
                grant,
                permit,
                reply,
            } => {
                if !reply.is_closed() {
                    let _ = reply.send(Ok(server.sync.voice_confirmation_actions(&grant, permit)));
                }
            }
            Self::Pair {
                grant,
                selector,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                let _ = reply.send(server.sync.prepare_voice_pair(&grant, selector));
            }
            Self::PairConfirmation {
                grant,
                selector,
                handshake,
                reply,
            } => {
                if !reply.is_closed() {
                    let _ = reply.send(
                        server
                            .sync
                            .voice_pair_confirmation(&grant, selector, handshake),
                    );
                }
            }
            Self::HighReady {
                grant,
                handshake,
                helpers,
                reply,
            } => {
                if !reply.is_closed() {
                    let result = match helpers {
                        [None, None] => {
                            server.sync.prepare_voice_high_ready(&grant, handshake, &[])
                        }
                        [Some(a), None] => {
                            server
                                .sync
                                .prepare_voice_high_ready(&grant, handshake, &[a])
                        }
                        [Some(a), Some(b)] => {
                            server
                                .sync
                                .prepare_voice_high_ready(&grant, handshake, &[a, b])
                        }
                        [None, Some(_)] => Err(VoiceError::Malformed),
                    };
                    let _ = reply.send(result);
                }
            }
            Self::InitialSdpIntent(command) => {
                if command.message.reply.is_closed() {
                    // Whole-wrapper Drop destroys grant/key/effect/reply before last work.
                    // This same path is exercised by the closed-receiver sentinel test.
                    drop(command);
                } else {
                    let sdp_rpc::Command { message, work } = command;
                    let InitialSdpMessage {
                        grant,
                        handshake,
                        key,
                        effect,
                        generation,
                        reply,
                    } = message;
                    let result = if work.matches_effect(&effect)
                        && work.covers_work(INITIAL_SDP_RPC_BYTES)
                    {
                        server.sync.prepare_voice_initial_sdp_intent(
                            &grant, handshake, key, effect, generation,
                        )
                    } else {
                        Err(VoiceError::Unauthorized)
                    };
                    drop(grant);
                    // No final Group/Policy guard survives this completion move, including
                    // rejection or a receiver disappearing during the Root publication.
                    sdp_rpc::complete(reply, work, |work| {
                        Ok(InitialSdpCompletion {
                            result,
                            _work: work,
                        })
                    });
                }
            }
            Self::HighReadyConnected {
                grant,
                handshake,
                reply,
            } => {
                if !reply.is_closed() {
                    let _ = reply.send(
                        server
                            .sync
                            .prepare_voice_high_ready_connected(&grant, handshake),
                    );
                }
            }
            Self::QueueReady {
                grant,
                handshake,
                reply,
            } => {
                if !reply.is_closed() {
                    let _ = reply.send(server.sync.queue_voice_high_ready(&grant, handshake));
                }
            }
            Self::PrepayLowSdp {
                grant,
                handshake,
                reply,
            } => {
                if !reply.is_closed() {
                    let _ = reply.send(server.sync.prepay_voice_low_sdp(&grant, handshake));
                }
            }
            Self::LowSdp {
                grant,
                handshake,
                reply,
            } => {
                if !reply.is_closed() {
                    let _ = reply.send(server.sync.prepare_voice_low_sdp(&grant, handshake));
                }
            }
            Self::Derive {
                grant,
                handshake,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                let _ = reply.send(server.sync.prepare_voice_derivation(&grant, handshake));
            }
            Self::Derived {
                grant,
                action,
                tag,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                let _ = reply.send(
                    server
                        .sync
                        .commit_voice_derivation_result(&grant, &action, tag),
                );
            }
            Self::ConfirmationResult {
                grant,
                action,
                body,
                tag,
                open,
                reply,
            } => {
                if reply.is_closed() {
                    // No Root effect. Actual payload and its payment drop together here,
                    // outside every Root/native lock, even for an abandoned queued request.
                    return;
                }
                // bytes() uses the pointer cached BEFORE admission, never owner AsRef. Both
                // Root methods independently validate the actual grant/action and output tag.
                let result = if open {
                    server
                        .sync
                        .commit_voice_confirmation_result(&grant, &action, body.bytes(), tag)
                } else {
                    server
                        .sync
                        .submit_voice_confirmation_result(&grant, &action, body.bytes(), tag)
                };
                // Preserve actual custody in completed/uncollected output. Failed send/drop
                // happens only after the synchronous Root turn released all protocol guards.
                let _ = reply.send(Ok(VoiceConfirmationCompletion::new(result, body)));
            }
            Self::StartConfirmation {
                grant,
                handshake,
                predecessor,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                let _ = reply.send(server.sync.start_voice_confirmation(
                    &grant,
                    handshake,
                    predecessor,
                ));
            }
            Self::RespondConfirmation {
                grant,
                handshake,
                trigger,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                let _ = reply.send(
                    server
                        .sync
                        .respond_voice_confirmation(&grant, handshake, &trigger),
                );
            }
        }
    }
}

impl ServerActor {
    /// Retire only the actual native call allocation. Safe after lease revocation and across
    /// lost replies; retrying an old stop cannot stop a newer endpoint in the same room.
    pub async fn stop_voice_endpoint(
        &self,
        grant: Arc<CallGrant>,
    ) -> Result<(), VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::Stop { grant, reply })
            .await
    }

    /// Trusted native rollout control only, never renderer data. The Root keeps Disabled as
    /// its default and the switch can only narrow authenticated group policy. Exact repeats
    /// are idempotent; a transition involving Direct revokes the old runtime policy lease.
    pub async fn set_voice_routing(
        &self,
        routing: catcoms_sync::VoiceRouting,
    ) -> Result<(), VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::Routing { routing, reply })
            .await
    }

    /// Fresh evidence from the actual participant's implicit watcher, bound to the original
    /// affine CallGrant allocation. No synthetic watcher or cached PairReceipt is substituted.
    pub async fn voice_participant_peers(
        &self,
        grant: Arc<CallGrant>,
    ) -> Result<Vec<FreshPeer>, VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::ParticipantPeers { grant, reply })
            .await
    }

    /// Observe proven peers with original bounded RPC/output custody. Cancellation drops
    /// only this receiver; the same permit remains with an admitted command or late reply.
    /// No browser metadata, pair factory, engine permit or native deadline is created here.
    pub async fn voice_participant_peer_snapshot(
        &self,
        grant: Arc<CallGrant>,
        permit: PeerSnapshotPermit,
    ) -> Result<PeerSnapshot, VoiceEndpointError> {
        let _waiter = permit.rpc_waiter().map_err(VoiceEndpointError::Protocol)?;
        self.voice_endpoint_request(|reply| Command::ParticipantPeerSnapshot {
            grant,
            permit,
            reply,
        })
        .await
    }
    /// Select the actual implicit watch's proven peer using immutable remote join/key identity.
    /// Lost replies retry this same selector; a peer rejoin cannot silently change the request.
    pub async fn prepare_voice_pair(
        &self,
        grant: Arc<CallGrant>,
        selector: PairSelector,
    ) -> Result<PairReceipt, VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::Pair {
            grant,
            selector,
            reply,
        })
        .await
    }
    /// Fixed local confirmation observation for the original grant, selector and handshake.
    /// Mailbox loss is uncertain metadata, never proof of establishment or a reason to create
    /// a replacement pair. Re-query the same identity; later SDP admission remains independent.
    pub async fn voice_pair_confirmation(
        &self,
        grant: Arc<CallGrant>,
        selector: PairSelector,
        handshake: Nonce,
    ) -> Result<VoicePairConfirmation, VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::PairConfirmation {
            grant,
            selector,
            handshake,
            reply,
        })
        .await
    }

    /// Prepare the original native HIGH receiver/Ready. No renderer packet, signer or
    /// clock enters this command. Retry only this same grant/handshake/helper choice.
    pub async fn prepare_voice_high_ready(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
        helpers: [Option<DeviceId>; 2],
    ) -> Result<bool, VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::HighReady {
            grant,
            handshake,
            helpers,
            reply,
        })
        .await
    }
    /// Native-only original phase/PC intent. The adapter must bind its actual key,
    /// storage and SDK owners before submitting; the result is not a serialized grant.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_voice_initial_sdp_intent(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
        key: catcoms_sync::voice_v2::chain::EstablishedKeyCustody,
        effect: catcoms_sync::voice_v2::chain::sdp_handoff::EffectObserver,
        generation: u64,
        work: catcoms_sync::voice_v2::chain::sdp_handoff::Copy,
    ) -> Result<InitialSdpCompletion, VoiceEndpointError> {
        self.queue_voice_initial_sdp_intent(grant, handshake, key, effect, generation, work)
            .await?
            .complete()
            .await
    }

    /// Enqueue with its same prepaid contribution before returning an observer. This is
    /// distinct from a possibly unpolled future and gives cancellation a precise boundary.
    #[allow(clippy::too_many_arguments)]
    pub async fn queue_voice_initial_sdp_intent(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
        key: catcoms_sync::voice_v2::chain::EstablishedKeyCustody,
        effect: catcoms_sync::voice_v2::chain::sdp_handoff::EffectObserver,
        generation: u64,
        work: catcoms_sync::voice_v2::chain::sdp_handoff::Copy,
    ) -> Result<InitialSdpPending, VoiceEndpointError> {
        if !work.matches_effect(&effect) || !work.covers_work(INITIAL_SDP_RPC_BYTES) {
            return Err(VoiceEndpointError::Protocol(VoiceError::Capacity));
        }
        let (command, pending) = sdp_rpc::request(work, |reply| InitialSdpMessage {
            grant,
            handshake,
            key,
            effect,
            generation,
            reply,
        });
        self.cmd_tx
            .send(AppCommand::VoiceEndpoint(VoiceEndpointCommand(
                Command::InitialSdpIntent(command),
            )))
            .await
            .map_err(|_| VoiceEndpointError::Stopped)?;
        Ok(InitialSdpPending(pending))
    }

    /// Select authenticated connected member helpers inside Root, then recover/prepare the
    /// same immutable HIGH Ready. The caller supplies no helper or version-compatibility hint.
    pub async fn prepare_voice_high_ready_connected(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
    ) -> Result<bool, VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::HighReadyConnected {
            grant,
            handshake,
            reply,
        })
        .await
    }
    /// Queue that exact prepared Ready. A lost reply recovers its original Source record,
    /// not another identity, origin debit or three-attempt allowance.
    pub async fn queue_voice_high_ready(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
    ) -> Result<bool, VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::QueueReady {
            grant,
            handshake,
            reply,
        })
        .await
    }
    /// Prepay original LOW capacity/endpoint records after native Finish. This alone does
    /// not accept Ready, stop Finish, adopt browser keys or authorize any SDP/RTC work.
    pub async fn prepay_voice_low_sdp(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
    ) -> Result<(), VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::PrepayLowSdp {
            grant,
            handshake,
            reply,
        })
        .await
    }
    /// Advance only actual native LOW Ready. `true` observes old Workflow drainage;
    /// `false` retains the same extracted owner while real old work still drains.
    /// Neither result grants SDP/browser/RTC activation. Capacity/Stopped retry the SAME
    /// owners; they never justify replacing the original preparation or old key Claim.
    pub async fn prepare_voice_low_sdp(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
    ) -> Result<bool, VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::LowSdp {
            grant,
            handshake,
            reply,
        })
        .await
    }

    async fn voice_endpoint_request<T: Send + 'static>(
        &self,
        build: impl FnOnce(Reply<T>) -> Command + Send,
    ) -> Result<T, VoiceEndpointError> {
        let (reply, result) = oneshot::channel();
        // This bounds admitted envelopes, not arbitrary waiting producers. The future native
        // renderer adapter must coalesce its own exact requests and prepay all IPC copy owners.
        self.cmd_tx
            .send(AppCommand::VoiceEndpoint(VoiceEndpointCommand(build(
                reply,
            ))))
            .await
            .map_err(|_| VoiceEndpointError::Stopped)?;
        result
            .await
            .map_err(|_| VoiceEndpointError::Stopped)?
            .map_err(VoiceEndpointError::Protocol)
    }

    /// Install the original native call's public join key. Retain the affine grant outside this
    /// request; the Root deliberately holds only a weak reference after the command completes.
    pub async fn start_voice_endpoint(
        &self,
        grant: Arc<CallGrant>,
        join: Nonce,
        public_key: [u8; 65],
    ) -> Result<(), VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::Start {
            grant,
            join,
            public_key,
            reply,
        })
        .await
    }

    /// Native controller/call parent retirement closes this observer without a later command.
    /// Losing its reply/last grant similarly closes it; the Root never owns affine reply custody.
    pub async fn start_voice_watch(
        &self,
        room: u128,
        parent: CallLease,
    ) -> Result<Arc<VoiceWatchGrant>, VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::Watch {
            room,
            parent,
            reply,
        })
        .await
    }

    /// Return native proven-peer owners from the exact original watcher. They retain their own
    /// observation charge; future serialized/browser copies need independent bounded custody.
    pub async fn voice_watch_peers(
        &self,
        grant: Arc<VoiceWatchGrant>,
    ) -> Result<Vec<FreshPeer>, VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::Peers { grant, reply })
            .await
    }

    /// One coalescing native revision observer. This is not payload delivery or an IPC registry;
    /// the production driver must retain one receiver instead of creating one on every UI poll.
    pub async fn voice_endpoint_changes(
        &self,
    ) -> Result<tokio::sync::watch::Receiver<u64>, VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::Changes { reply })
            .await
    }

    /// Original pending actions from the exact native participant. The affine prepaid owner
    /// transfers into the command and then its actual output, including a Root refusal. Caller
    /// cancellation cannot release the32-handle snapshot/oneshot payment ahead of collection.
    /// This rotates observation, never consumes tokens, admits engines or reconstructs keys.
    pub async fn voice_confirmation_actions(
        &self,
        grant: Arc<CallGrant>,
        permit: CryptoActionSnapshotPermit,
    ) -> Result<CryptoActionSnapshot, VoiceEndpointError> {
        // Must precede oneshot allocation: stop can destroy the queued command while this
        // actual receiver remains unpolled; cancellation can destroy this waiter first.
        let _waiter = permit.rpc_waiter().map_err(VoiceEndpointError::Protocol)?;
        self.voice_endpoint_request(|reply| Command::ConfirmationActions {
            grant,
            permit,
            reply,
        })
        .await
    }

    /// Recover the original native derivation row; neither a handshake ID nor a cached receipt
    /// can create a replacement call, pending pair or browser key owner.
    pub async fn prepare_voice_derivation(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
    ) -> Result<Arc<CryptoAction>, VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::Derive {
            grant,
            handshake,
            reply,
        })
        .await
    }

    /// Submit only the empty completion marker for an actual registered Derive action.
    pub async fn commit_voice_derivation_result(
        &self,
        grant: Arc<CallGrant>,
        action: Arc<CryptoAction>,
        tag: [u8; 32],
    ) -> Result<CryptoResultReceipt, VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::Derived {
            grant,
            action,
            tag,
            reply,
        })
        .await
    }

    /// Submit the ORIGINAL native-owned sealed result without copying its byte buffer. The
    /// caller keeps this same Body through retryable/uncertain failures. Its owner must prepay
    /// the payload and actor metadata before this method, not borrow ordinary capacity later.
    /// This trusted Rust seam creates no renderer command, byte channel or live dispatch.
    pub async fn submit_voice_confirmation_result(
        &self,
        grant: Arc<CallGrant>,
        action: Arc<CryptoAction>,
        body: &VoiceConfirmationBody,
        tag: [u8; 32],
    ) -> Result<VoiceConfirmationCompletion, VoiceEndpointError> {
        let body = body.claim().map_err(VoiceEndpointError::Protocol)?;
        self.voice_endpoint_request(|reply| Command::ConfirmationResult {
            grant,
            action,
            body,
            tag,
            open: false,
            reply,
        })
        .await
    }

    /// Commit an exact opened result through the existing Root checks. No plaintext is
    /// serialized or copied to the reply. Completion still holds the same actual body until
    /// collection; the small Root outcome is not a native/browser drainage assertion.
    pub async fn commit_voice_confirmation_result(
        &self,
        grant: Arc<CallGrant>,
        action: Arc<CryptoAction>,
        body: &VoiceConfirmationBody,
        tag: [u8; 32],
    ) -> Result<VoiceConfirmationCompletion, VoiceEndpointError> {
        let body = body.claim().map_err(VoiceEndpointError::Protocol)?;
        self.voice_endpoint_request(|reply| Command::ConfirmationResult {
            grant,
            action,
            body,
            tag,
            open: true,
            reply,
        })
        .await
    }

    /// None identifies the original start; Some names the exact native predecessor's successor.
    /// Retry the same request after uncertainty instead of allocating another candidate.
    pub async fn start_voice_confirmation(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
        predecessor: Option<Nonce>,
    ) -> Result<Arc<CryptoAction>, VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::StartConfirmation {
            grant,
            handshake,
            predecessor,
            reply,
        })
        .await
    }

    /// Choose Proof/Finish only from the exact native retained accepted trigger action.
    pub async fn respond_voice_confirmation(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
        trigger: Arc<CryptoAction>,
    ) -> Result<Arc<CryptoAction>, VoiceEndpointError> {
        self.voice_endpoint_request(|reply| Command::RespondConfirmation {
            grant,
            handshake,
            trigger,
            reply,
        })
        .await
    }
}

#[cfg(test)]
mod tests;

/// Fixed queued command, outer App envelope, completion and bounded oneshot bookkeeping.
/// Native adds the actual calling future before admitting its original phase Copy.
pub const INITIAL_SDP_RPC_BYTES: usize = std::mem::size_of::<Command>()
    + std::mem::size_of::<AppCommand>()
    + std::mem::size_of::<InitialSdpCompletion>()
    + std::mem::size_of::<InitialSdpPending>()
    + std::mem::size_of::<InitialSdpWork>()
    + 2 * std::mem::size_of::<usize>()
    + 512;
