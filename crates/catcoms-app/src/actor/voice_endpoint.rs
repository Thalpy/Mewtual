//! Trusted, body-free native voice controls through the existing bounded actor mailbox. Root
//! still owns all protocol state and authorizes every first effect in its serialized turn.
//! This module exposes no Tauri command, browser projection, plaintext or live V2 dispatch.
use super::*;
use catcoms_sync::voice_v2::crypto_actions::{CryptoAction, CryptoResultReceipt};
use catcoms_sync::voice_v2::presence::FreshPeer;
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

    /// Submit only the empty completion marker for an actual registered Derive action. There
    /// are deliberately no seal/open byte commands before IPC/queue copy custody is prepaid.
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
