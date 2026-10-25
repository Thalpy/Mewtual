//! Typed Rust-only SDP RPCs. Every queued command/pending/completion shares its original
//! paid work, including native publication custody. This is not a renderer command grammar.
use super::sdp_body::{
    VoiceSdpBody, VoiceSdpCompletion, VoiceSdpPending, Work, VOICE_SDP_RPC_METADATA_BYTES,
};
use super::*;
use catcoms_sync::voice_v2::chain::sdp_handoff::Copy;
use catcoms_sync::voice_v2::sdp::publication::Writer;
use catcoms_sync::voice_v2::sdp::{InitialSdpIntent, SdpDelivery, SdpOpenOutput, SdpOpenResult};

pub(super) enum Message {
    LowOffer {
        grant: Arc<CallGrant>,
        handshake: Nonce,
        intent: Arc<InitialSdpIntent>,
        outcome: Option<Writer>,
        reply: Reply<VoiceSdpCompletion<Arc<CryptoAction>>>,
    },
    HighAnswer {
        grant: Arc<CallGrant>,
        handshake: Nonce,
        intent: Arc<InitialSdpIntent>,
        delivery: Arc<SdpDelivery>,
        outcome: Option<Writer>,
        reply: Reply<VoiceSdpCompletion<Arc<CryptoAction>>>,
    },
    OpenAction {
        grant: Arc<CallGrant>,
        handshake: Nonce,
        intent: Arc<InitialSdpIntent>,
        outcome: Option<Writer>,
        reply: Reply<VoiceSdpCompletion<Arc<CryptoAction>>>,
    },
    Sealed {
        grant: Arc<CallGrant>,
        intent: Arc<InitialSdpIntent>,
        action: Arc<CryptoAction>,
        tag: [u8; 32],
        reply: Reply<VoiceSdpCompletion<CryptoResultReceipt>>,
    },
    Opened {
        grant: Arc<CallGrant>,
        intent: Arc<InitialSdpIntent>,
        action: Arc<CryptoAction>,
        tag: [u8; 32],
        reply: Reply<VoiceSdpCompletion<SdpOpenResult>>,
    },
    AuthenticationFailed {
        grant: Arc<CallGrant>,
        intent: Arc<InitialSdpIntent>,
        action: Arc<CryptoAction>,
        reply: Reply<VoiceSdpCompletion<SdpOpenResult>>,
    },
    Delivery {
        grant: Arc<CallGrant>,
        intent: Arc<InitialSdpIntent>,
        action: Arc<CryptoAction>,
        delivery: Arc<SdpDelivery>,
        applied: bool,
        reply: Reply<VoiceSdpCompletion<()>>,
    },
}
impl Message {
    fn abandoned(&self) -> bool {
        match self {
            Self::LowOffer { reply, .. }
            | Self::HighAnswer { reply, .. }
            | Self::OpenAction { reply, .. } => reply.is_closed(),
            Self::Sealed { reply, .. } => reply.is_closed(),
            Self::Opened { reply, .. } | Self::AuthenticationFailed { reply, .. } => {
                reply.is_closed()
            }
            Self::Delivery { reply, .. } => reply.is_closed(),
        }
    }
}
pub(super) type Command = sdp_rpc::Command<Message, Work>;
type RecordedRequest = Result<
    (
        Command,
        sdp_rpc::Pending<Result<VoiceSdpCompletion<Arc<CryptoAction>>, VoiceError>, Work>,
    ),
    VoiceEndpointError,
>;

// Eager constructors execute before the returned future exists. Inventory their actual
// parameter/closure/request wrapper types separately: future sizeof alone would omit
// this synchronous ownership graph. Count the caller and its ONE moved request pair,
// plus request()'s intermediate Work/Arc/message/channel handles. `prepared` moves into
// the async sender; it never coexists with a second owned Command/Pending pair. Compiler
// move temporaries are not additional owned graphs. Heap backing and Tokio
// allocation bookkeeping are already included in sdp_body's fixed graph. This is bounded
// owned storage, not a claim about compiler-specific machine stack or runtime call depth.
struct RecordedConstruction {
    _request: RecordedRequest,
    _caller: RecordedCaller,
    _work: Work,
    _anchor: Arc<Work>,
    _message: Message,
    _reply: Reply<VoiceSdpCompletion<Arc<CryptoAction>>>,
    _receiver: oneshot::Receiver<Result<VoiceSdpCompletion<Arc<CryptoAction>>, VoiceError>>,
}
struct RecordedCaller {
    _actor: &'static ServerActor,
    _grant: Arc<CallGrant>,
    _handshake: Nonce,
    _intent: Arc<InitialSdpIntent>,
    _delivery: Option<Arc<SdpDelivery>>,
    _copy: Option<Copy>,
    _body: Option<&'static VoiceSdpBody>,
    _writer: Writer,
}
pub(super) const RECORDED_CONSTRUCTION_BYTES: usize = std::mem::size_of::<RecordedConstruction>();
pub(super) fn apply<T: MeshTransport, R: CryptoRngCore>(
    command: Command,
    server: &mut Server<T, R>,
) {
    if command.message.abandoned() {
        drop(command);
        return;
    }
    let sdp_rpc::Command { message, work } = command;
    // Destructuring ends the wrapper's automatic field-order protection. Each arm must
    // explicitly retire argument owners before the final shared work can disappear.
    match message {
        Message::LowOffer {
            grant,
            handshake,
            intent,
            mut outcome,
            reply,
        } => {
            let result = if work.uses_intent(&intent) {
                work.bytes().and_then(|bytes| match outcome.as_mut() {
                    Some(outcome) => server.sync.prepare_voice_low_sdp_offer_recorded(
                        &grant, handshake, &intent, bytes, outcome,
                    ),
                    None => server
                        .sync
                        .prepare_voice_low_sdp_offer(&grant, handshake, &intent, bytes),
                })
            } else {
                Err(VoiceError::Unauthorized)
            };
            // Writer retirement must precede Work/Copy collection and its cleanup wake,
            // including refusal before Root entry. A missing reply is never a negative.
            drop((outcome, grant, intent));
            sdp_rpc::complete(reply, work, |work| {
                Ok(VoiceSdpCompletion::new(result, work))
            });
        }
        Message::HighAnswer {
            grant,
            handshake,
            intent,
            delivery,
            mut outcome,
            reply,
        } => {
            let result = if work.uses_intent(&intent) {
                work.bytes().and_then(|bytes| match outcome.as_mut() {
                    Some(outcome) => server.sync.prepare_voice_high_sdp_answer_recorded(
                        &grant, handshake, &intent, &delivery, bytes, outcome,
                    ),
                    None => server.sync.prepare_voice_high_sdp_answer(
                        &grant, handshake, &intent, &delivery, bytes,
                    ),
                })
            } else {
                Err(VoiceError::Unauthorized)
            };
            drop((outcome, grant, intent, delivery));
            sdp_rpc::complete(reply, work, |work| {
                Ok(VoiceSdpCompletion::new(result, work))
            });
        }
        Message::OpenAction {
            grant,
            handshake,
            intent,
            mut outcome,
            reply,
        } => {
            let result = if work.uses_intent(&intent) {
                match outcome.as_mut() {
                    Some(outcome) => server
                        .sync
                        .prepare_voice_sdp_open_recorded(&grant, handshake, &intent, outcome),
                    None => server
                        .sync
                        .prepare_voice_sdp_open(&grant, handshake, &intent),
                }
            } else {
                Err(VoiceError::Unauthorized)
            };
            drop((outcome, grant, intent));
            sdp_rpc::complete(reply, work, |work| {
                Ok(VoiceSdpCompletion::new(result, work))
            });
        }
        Message::Sealed {
            grant,
            intent,
            action,
            tag,
            reply,
        } => {
            let result = if work.uses_intent(&intent) && action.uses_sdp_intent(&intent) {
                work.bytes().and_then(|bytes| {
                    server
                        .sync
                        .submit_voice_sdp_result(&grant, &action, bytes, tag)
                })
            } else {
                Err(VoiceError::Unauthorized)
            };
            drop((grant, intent, action));
            sdp_rpc::complete(reply, work, |work| {
                Ok(VoiceSdpCompletion::new(result, work))
            });
        }
        Message::Opened {
            grant,
            intent,
            action,
            tag,
            reply,
        } => {
            let result = if work.uses_intent(&intent) && action.uses_sdp_intent(&intent) {
                work.bytes().and_then(|bytes| {
                    server.sync.submit_voice_sdp_open_result(
                        &grant,
                        &action,
                        SdpOpenOutput::Plaintext {
                            bytes,
                            claimed_tag: tag,
                        },
                    )
                })
            } else {
                Err(VoiceError::Unauthorized)
            };
            drop((grant, intent, action));
            sdp_rpc::complete(reply, work, |work| {
                Ok(VoiceSdpCompletion::new(result, work))
            });
        }
        Message::AuthenticationFailed {
            grant,
            intent,
            action,
            reply,
        } => {
            let result = if work.uses_intent(&intent) && action.uses_sdp_intent(&intent) {
                server.sync.submit_voice_sdp_open_result(
                    &grant,
                    &action,
                    SdpOpenOutput::AuthenticationFailed,
                )
            } else {
                Err(VoiceError::Unauthorized)
            };
            drop((grant, intent, action));
            sdp_rpc::complete(reply, work, |work| {
                Ok(VoiceSdpCompletion::new(result, work))
            });
        }
        Message::Delivery {
            grant,
            intent,
            action,
            delivery,
            applied,
            reply,
        } => {
            let result = if !(work.uses_intent(&intent) && action.uses_sdp_intent(&intent)) {
                Err(VoiceError::Unauthorized)
            } else if applied {
                server
                    .sync
                    .complete_voice_sdp_delivery(&grant, &action, &delivery)
            } else {
                server
                    .sync
                    .admit_voice_sdp_delivery(&grant, &action, &delivery)
            };
            drop((grant, intent, action, delivery));
            sdp_rpc::complete(reply, work, |work| {
                Ok(VoiceSdpCompletion::new(result, work))
            });
        }
    }
}
fn action_intent(
    action: &Arc<CryptoAction>,
    intent: &Arc<InitialSdpIntent>,
) -> Result<(), VoiceEndpointError> {
    if action.uses_sdp_intent(intent) {
        Ok(())
    } else {
        Err(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
    }
}
fn metadata(copy: Copy, intent: &Arc<InitialSdpIntent>) -> Result<Work, VoiceEndpointError> {
    check_metadata(&copy, intent)?;
    Ok(Work::Metadata(copy))
}
fn check_metadata(copy: &Copy, intent: &Arc<InitialSdpIntent>) -> Result<(), VoiceEndpointError> {
    if !copy.uses_intent(intent) || !copy.has_keepalive() {
        return Err(VoiceEndpointError::Protocol(VoiceError::Unauthorized));
    }
    if !copy.covers_work(VOICE_SDP_RPC_METADATA_BYTES) {
        return Err(VoiceEndpointError::Protocol(VoiceError::Capacity));
    }
    Ok(())
}
impl ServerActor {
    /// Callers synchronously construct the whole Command/Pending graph BEFORE invoking
    /// this sender. An unpolled or cancelled future must retire Writer before the last
    /// paid Copy can notify collection; independent async parameters are insufficient.
    async fn queue_sdp_recorded(
        &self,
        prepared: RecordedRequest,
    ) -> Result<VoiceSdpPending<Arc<CryptoAction>>, VoiceEndpointError> {
        let (command, pending) = prepared?;
        self.cmd_tx
            .send(AppCommand::VoiceEndpoint(VoiceEndpointCommand(
                super::Command::Sdp(command),
            )))
            .await
            .map_err(|_| VoiceEndpointError::Stopped)?;
        Ok(VoiceSdpPending(pending))
    }

    /// Publish one original LOW action with its retained native outcome. Calling this
    /// function immediately claims the body and takes the affine Writer; never recreate
    /// either on a lost reply. History recovery belongs to the original native slot.
    pub fn queue_voice_sdp_offer_recorded(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
        intent: Arc<InitialSdpIntent>,
        body: &VoiceSdpBody,
        outcome: Writer,
    ) -> impl std::future::Future<
        Output = Result<VoiceSdpPending<Arc<CryptoAction>>, VoiceEndpointError>,
    > + '_ {
        let prepared = match body.claim(&intent, catcoms_sync::voice_v2::MAX_SDP_PLAINTEXT) {
            Ok(flight) => Ok(sdp_rpc::request(Work::Body(flight), |reply| {
                Message::LowOffer {
                    grant,
                    handshake,
                    intent,
                    outcome: Some(outcome),
                    reply,
                }
            })),
            Err(reason) => {
                drop(outcome);
                drop((grant, intent));
                Err(VoiceEndpointError::Protocol(reason))
            }
        };
        self.queue_sdp_recorded(prepared)
    }

    /// HIGH's recorded publication still requires the SAME actually Applied delivery.
    /// Like LOW, command construction is eager even if its returned future is never polled.
    pub fn queue_voice_sdp_answer_recorded(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
        intent: Arc<InitialSdpIntent>,
        delivery: Arc<SdpDelivery>,
        body: &VoiceSdpBody,
        outcome: Writer,
    ) -> impl std::future::Future<
        Output = Result<VoiceSdpPending<Arc<CryptoAction>>, VoiceEndpointError>,
    > + '_ {
        let prepared = match body.claim(&intent, catcoms_sync::voice_v2::MAX_SDP_PLAINTEXT) {
            Ok(flight) => Ok(sdp_rpc::request(Work::Body(flight), |reply| {
                Message::HighAnswer {
                    grant,
                    handshake,
                    intent,
                    delivery,
                    outcome: Some(outcome),
                    reply,
                }
            })),
            Err(reason) => {
                drop(outcome);
                drop((grant, intent, delivery));
                Err(VoiceEndpointError::Protocol(reason))
            }
        };
        self.queue_sdp_recorded(prepared)
    }

    /// Resolve native's actual admitted Open input with one original outcome. Refusal
    /// retires Writer before Copy, so a CLOSED+zero waiter cannot see stale InFlight.
    pub fn queue_voice_sdp_open_recorded(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
        intent: Arc<InitialSdpIntent>,
        copy: Copy,
        outcome: Writer,
    ) -> impl std::future::Future<
        Output = Result<VoiceSdpPending<Arc<CryptoAction>>, VoiceEndpointError>,
    > + '_ {
        let prepared = match check_metadata(&copy, &intent) {
            Ok(()) => Ok(sdp_rpc::request(Work::Metadata(copy), |reply| {
                Message::OpenAction {
                    grant,
                    handshake,
                    intent,
                    outcome: Some(outcome),
                    reply,
                }
            })),
            Err(reason) => {
                drop(outcome);
                drop(copy);
                drop((grant, intent));
                Err(reason)
            }
        };
        self.queue_sdp_recorded(prepared)
    }

    async fn queue_sdp<T>(
        &self,
        work: Work,
        message: impl FnOnce(Reply<VoiceSdpCompletion<T>>) -> Message,
    ) -> Result<VoiceSdpPending<T>, VoiceEndpointError> {
        let (command, pending) = sdp_rpc::request(work, message);
        self.cmd_tx
            .send(AppCommand::VoiceEndpoint(VoiceEndpointCommand(
                super::Command::Sdp(command),
            )))
            .await
            .map_err(|_| VoiceEndpointError::Stopped)?;
        Ok(VoiceSdpPending(pending))
    }
    /// Queue the native-chosen LOW Offer with its exact original paid plaintext body.
    /// The same Body must be retained through uncertain retries; do not repeat crypto.
    pub async fn queue_voice_sdp_offer(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
        intent: Arc<InitialSdpIntent>,
        body: &VoiceSdpBody,
    ) -> Result<VoiceSdpPending<Arc<CryptoAction>>, VoiceEndpointError> {
        let work = Work::Body(
            body.claim(&intent, catcoms_sync::voice_v2::MAX_SDP_PLAINTEXT)
                .map_err(VoiceEndpointError::Protocol)?,
        );
        self.queue_sdp(work, |reply| Message::LowOffer {
            grant,
            handshake,
            intent,
            outcome: None,
            reply,
        })
        .await
    }
    /// Only the original applied remote description can authorize this HIGH Answer.
    pub async fn queue_voice_sdp_answer(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
        intent: Arc<InitialSdpIntent>,
        delivery: Arc<SdpDelivery>,
        body: &VoiceSdpBody,
    ) -> Result<VoiceSdpPending<Arc<CryptoAction>>, VoiceEndpointError> {
        let work = Work::Body(
            body.claim(&intent, catcoms_sync::voice_v2::MAX_SDP_PLAINTEXT)
                .map_err(VoiceEndpointError::Protocol)?,
        );
        self.queue_sdp(work, |reply| Message::HighAnswer {
            grant,
            handshake,
            intent,
            delivery,
            outcome: None,
            reply,
        })
        .await
    }
    /// Resolve actual already-admitted Offer/Answer input; no renderer token chooses it.
    pub async fn queue_voice_sdp_open(
        &self,
        grant: Arc<CallGrant>,
        handshake: Nonce,
        intent: Arc<InitialSdpIntent>,
        copy: Copy,
    ) -> Result<VoiceSdpPending<Arc<CryptoAction>>, VoiceEndpointError> {
        let work = metadata(copy, &intent)?;
        self.queue_sdp(work, |reply| Message::OpenAction {
            grant,
            handshake,
            intent,
            outcome: None,
            reply,
        })
        .await
    }
    /// Submit exact sealed bytes once; completion retains their actual paid Buffer.
    pub async fn queue_voice_sdp_sealed_result(
        &self,
        grant: Arc<CallGrant>,
        intent: Arc<InitialSdpIntent>,
        action: Arc<CryptoAction>,
        body: &VoiceSdpBody,
        tag: [u8; 32],
    ) -> Result<VoiceSdpPending<CryptoResultReceipt>, VoiceEndpointError> {
        action_intent(&action, &intent)?;
        let work = Work::Body(
            body.claim(&intent, catcoms_sync::voice_v2::MAX_SDP_PLAINTEXT + 16)
                .map_err(VoiceEndpointError::Protocol)?,
        );
        self.queue_sdp(work, |reply| Message::Sealed {
            grant,
            intent,
            action,
            tag,
            reply,
        })
        .await
    }
    /// Commit the original SDK plaintext without a Root/Actor result copy. Recovery after
    /// End belongs to the authenticated native registered action before this current path.
    pub async fn queue_voice_sdp_opened_result(
        &self,
        grant: Arc<CallGrant>,
        intent: Arc<InitialSdpIntent>,
        action: Arc<CryptoAction>,
        body: &VoiceSdpBody,
        tag: [u8; 32],
    ) -> Result<VoiceSdpPending<SdpOpenResult>, VoiceEndpointError> {
        action_intent(&action, &intent)?;
        let work = Work::Body(
            body.claim(&intent, catcoms_sync::voice_v2::MAX_SDP_PLAINTEXT)
                .map_err(VoiceEndpointError::Protocol)?,
        );
        self.queue_sdp(work, |reply| Message::Opened {
            grant,
            intent,
            action,
            tag,
            reply,
        })
        .await
    }
    /// Closed AEAD failure, with separately paid metadata and no fabricated plaintext.
    pub async fn queue_voice_sdp_authentication_failed(
        &self,
        grant: Arc<CallGrant>,
        intent: Arc<InitialSdpIntent>,
        action: Arc<CryptoAction>,
        copy: Copy,
    ) -> Result<VoiceSdpPending<SdpOpenResult>, VoiceEndpointError> {
        action_intent(&action, &intent)?;
        let work = metadata(copy, &intent)?;
        self.queue_sdp(work, |reply| Message::AuthenticationFailed {
            grant,
            intent,
            action,
            reply,
        })
        .await
    }
    /// Fresh admission names one original logical RTC effect; retry never re-arms SDK.
    pub async fn queue_voice_sdp_delivery_admission(
        &self,
        grant: Arc<CallGrant>,
        intent: Arc<InitialSdpIntent>,
        action: Arc<CryptoAction>,
        delivery: Arc<SdpDelivery>,
        copy: Copy,
    ) -> Result<VoiceSdpPending<()>, VoiceEndpointError> {
        action_intent(&action, &intent)?;
        let work = metadata(copy, &intent)?;
        self.queue_sdp(work, |reply| Message::Delivery {
            grant,
            intent,
            action,
            delivery,
            applied: false,
            reply,
        })
        .await
    }
    /// Invoke ONLY after the original successful private SDK await. Root records ordering;
    /// it does not prove SDK execution or return native/browser custody before actual Drop.
    pub async fn queue_voice_sdp_delivery_completion(
        &self,
        grant: Arc<CallGrant>,
        intent: Arc<InitialSdpIntent>,
        action: Arc<CryptoAction>,
        delivery: Arc<SdpDelivery>,
        copy: Copy,
    ) -> Result<VoiceSdpPending<()>, VoiceEndpointError> {
        action_intent(&action, &intent)?;
        let work = metadata(copy, &intent)?;
        self.queue_sdp(work, |reply| Message::Delivery {
            grant,
            intent,
            action,
            delivery,
            applied: true,
            reply,
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // No synthetic Phase/Intent/Action/Delivery is constructed. Type inference measures the
    // compiler's actual anonymous future for each public method using uncalled closures.
    // 'static borrows affect lifetime only, not layout. Each future retains exactly the
    // same owned parameters it would at the real native call boundary.
    type Actor = &'static ServerActor;
    type Body = &'static VoiceSdpBody;
    fn measured<F>(
        _make: impl FnOnce(
            Actor,
            Arc<CallGrant>,
            Arc<InitialSdpIntent>,
            Arc<CryptoAction>,
            Arc<SdpDelivery>,
            Copy,
            Body,
        ) -> F,
    ) -> usize {
        std::mem::size_of::<F>()
    }
    #[test]
    fn all_concrete_sdp_rpc_futures_fit_the_original_body_or_metadata_contribution() {
        fn recorded<F>(
            _make: impl FnOnce(
                Actor,
                Arc<CallGrant>,
                Arc<InitialSdpIntent>,
                Arc<SdpDelivery>,
                Copy,
                Body,
                Writer,
            ) -> F,
        ) -> usize {
            std::mem::size_of::<F>()
        }
        let futures = [
            recorded(|actor, grant, intent, _, _, body, writer| {
                actor.queue_voice_sdp_offer_recorded(grant, [0; 32], intent, body, writer)
            }),
            recorded(|actor, grant, intent, delivery, _, body, writer| {
                actor
                    .queue_voice_sdp_answer_recorded(grant, [0; 32], intent, delivery, body, writer)
            }),
            recorded(|actor, grant, intent, _, copy, _, writer| {
                actor.queue_voice_sdp_open_recorded(grant, [0; 32], intent, copy, writer)
            }),
            measured(|actor, grant, intent, _, _, _, body| {
                actor.queue_voice_sdp_offer(grant, [0; 32], intent, body)
            }),
            measured(|actor, grant, intent, _, delivery, _, body| {
                actor.queue_voice_sdp_answer(grant, [0; 32], intent, delivery, body)
            }),
            measured(|actor, grant, intent, _, _, copy, _| {
                actor.queue_voice_sdp_open(grant, [0; 32], intent, copy)
            }),
            measured(|actor, grant, intent, action, _, _, body| {
                actor.queue_voice_sdp_sealed_result(grant, intent, action, body, [0; 32])
            }),
            measured(|actor, grant, intent, action, _, _, body| {
                actor.queue_voice_sdp_opened_result(grant, intent, action, body, [0; 32])
            }),
            measured(|actor, grant, intent, action, _, copy, _| {
                actor.queue_voice_sdp_authentication_failed(grant, intent, action, copy)
            }),
            measured(|actor, grant, intent, action, delivery, copy, _| {
                actor.queue_voice_sdp_delivery_admission(grant, intent, action, delivery, copy)
            }),
            measured(|actor, grant, intent, action, delivery, copy, _| {
                actor.queue_voice_sdp_delivery_completion(grant, intent, action, delivery, copy)
            }),
        ];
        for future in futures {
            let total = super::super::sdp_body::voice_sdp_rpc_bytes(future);
            assert!(
                total <= VOICE_SDP_RPC_METADATA_BYTES,
                "SDP RPC graph needs {total}"
            );
        }
        fn completion<F, T>(_make: impl FnOnce(VoiceSdpPending<T>) -> F) -> usize {
            std::mem::size_of::<F>()
        }
        for future in [
            completion(|pending: VoiceSdpPending<Arc<CryptoAction>>| pending.complete()),
            completion(|pending: VoiceSdpPending<CryptoResultReceipt>| pending.complete()),
            completion(|pending: VoiceSdpPending<SdpOpenResult>| pending.complete()),
            completion(|pending: VoiceSdpPending<()>| pending.complete()),
        ] {
            let total = super::super::sdp_body::voice_sdp_rpc_bytes(future);
            assert!(
                total <= VOICE_SDP_RPC_METADATA_BYTES,
                "SDP collection graph needs {total}"
            );
        }
    }
}
