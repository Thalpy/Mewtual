//! One bounded ordinary-member gossip receiver on the original native mesh.
use std::sync::{Arc, Weak};

use bytes::Bytes;
use catcoms_rt::{ConnectionLease, PeerId, RequestCancellation, Topic, MAX_PUBLISH_ONCE_BYTES};
use tokio::sync::{mpsc, oneshot, watch, OwnedSemaphorePermit, Semaphore};

use crate::{event_dispatch, Command};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NativeChatMemberGossipError {
    #[error("the original gossip receiver is busy")]
    Busy,
    #[error("invalid member gossip topic or frame bound")]
    Bounds,
    #[error("the original member gossip receiver retired")]
    Revoked,
    #[error("the original native mesh closed")]
    Closed,
    #[error("the native mesh refused to subscribe")]
    Subscribe,
}

/// Weak capture of one native actor. It holds no event inbox or actor owner.
#[derive(Clone)]
pub struct NativeChatMemberIngress {
    pub(super) commands: mpsc::WeakSender<Command>,
    pub(super) slot: Weak<Semaphore>,
}

impl NativeChatMemberIngress {
    /// Subscribe to the host's exact blinded topic. The host supplies and funds
    /// the original selected lifetime and frame bound before capturing this.
    pub async fn open(
        self,
        topic: Topic,
        maximum: usize,
        scope: RequestCancellation,
    ) -> Result<NativeChatMemberGossip, NativeChatMemberGossipError> {
        use NativeChatMemberGossipError as Error;
        if topic.as_bytes().len() != 32 || maximum == 0 || maximum > MAX_PUBLISH_ONCE_BYTES {
            return Err(Error::Bounds);
        }
        if scope.is_cancelled() {
            return Err(Error::Revoked);
        }
        let permit = self
            .slot
            .upgrade()
            .ok_or(Error::Closed)?
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let commands = self.commands.upgrade().ok_or(Error::Closed)?;
        let (owner, negative) = watch::channel(());
        let original = Arc::new(Original {
            negative,
            scope,
            slot: self.slot,
        });
        let (sender, receiver) = mpsc::channel(1);
        let (loss, loss_receiver) = watch::channel(0);
        let (reply, opened) = oneshot::channel();
        let registration = Registration {
            topic: Topic::new(Bytes::copy_from_slice(topic.as_bytes())),
            maximum,
            original: original.clone(),
            sender,
            loss,
            _permit: permit,
        };
        commands
            .try_send(Command::OpenNativeChatMemberGossip {
                registration,
                reply,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => Error::Busy,
                mpsc::error::TrySendError::Closed(_) => Error::Closed,
            })?;
        drop(commands);
        opened.await.map_err(|_| Error::Closed)??;
        if !original.is_current_cached() {
            return Err(Error::Revoked);
        }
        Ok(NativeChatMemberGossip {
            _owner: owner,
            receiver,
            loss: loss_receiver,
            original,
        })
    }

    pub fn owns_frame(&self, frame: &NativeChatMemberFrame) -> bool {
        self.slot.ptr_eq(&frame.observation.original.slot) && frame.observation.is_current_cached()
    }
}

pub(super) struct Original {
    negative: watch::Receiver<()>,
    scope: RequestCancellation,
    slot: Weak<Semaphore>,
}

impl Original {
    pub(super) fn is_current_cached(&self) -> bool {
        self.negative.has_changed().is_ok()
            && self.slot.strong_count() != 0
            && !self.scope.is_cancelled()
    }

    pub(super) async fn retired(&self) {
        let mut negative = self.negative.clone();
        let mut scope = self.scope.clone();
        tokio::select! {
            _ = negative.changed() => {},
            _ = scope.cancelled() => {},
        }
    }
}

/// Original subscribed inbox. Dropping it revokes already handed-off packets
/// before dropping queued data. The actor removes this subscription on wake.
pub struct NativeChatMemberGossip {
    _owner: watch::Sender<()>,
    receiver: mpsc::Receiver<NativeChatMemberFrame>,
    loss: watch::Receiver<u64>,
    original: Arc<Original>,
}

pub enum NativeChatMemberGossipEvent {
    Frame(NativeChatMemberFrame),
    RecoveryNeeded,
}

impl NativeChatMemberGossip {
    pub async fn next(&mut self) -> Option<NativeChatMemberGossipEvent> {
        tokio::select! {
            biased;
            _ = self.original.retired() => None,
            result = self.loss.changed() => result.ok().map(|()| NativeChatMemberGossipEvent::RecoveryNeeded),
            frame = self.receiver.recv() => frame.map(NativeChatMemberGossipEvent::Frame),
        }
    }
}

/// Bounded ciphertext on the original authenticated carrier and registration.
/// The mesh origin is routing data. Core still verifies the signed author.
pub struct NativeChatMemberFrame {
    topic: Topic,
    from: PeerId,
    data: Bytes,
    observation: NativeChatMemberFrameObservation,
}

impl NativeChatMemberFrame {
    pub fn observe(&self) -> NativeChatMemberFrameObservation {
        self.observation.clone()
    }

    pub fn current_data(&self) -> Option<(&Topic, PeerId, &[u8])> {
        self.observation
            .is_current_cached()
            .then_some((&self.topic, self.from, self.data.as_ref()))
    }
}

/// The exact receive epoch and carrier negative, with no ciphertext lease.
#[derive(Clone)]
pub struct NativeChatMemberFrameObservation {
    carrier: ConnectionLease,
    original: Arc<Original>,
}

impl NativeChatMemberFrameObservation {
    pub fn is_current_cached(&self) -> bool {
        self.original.is_current_cached() && self.carrier.is_live()
    }
}

pub(super) struct Registration {
    pub(super) topic: Topic,
    maximum: usize,
    pub(super) original: Arc<Original>,
    sender: mpsc::Sender<NativeChatMemberFrame>,
    loss: watch::Sender<u64>,
    _permit: OwnedSemaphorePermit,
}

impl Registration {
    pub(super) fn deliver(
        &self,
        from: PeerId,
        data: Vec<u8>,
        carrier: Option<ConnectionLease>,
        budget: &Arc<event_dispatch::Budget>,
    ) {
        if !self.original.is_current_cached() || data.len() > self.maximum {
            return;
        }
        let delivered = (|| {
            let carrier = carrier.filter(ConnectionLease::is_live)?;
            let queued = self.sender.try_reserve().ok()?;
            let credit = budget.charge(data.len(), None)?;
            queued.send(NativeChatMemberFrame {
                topic: self.topic.clone(),
                from,
                data: event_dispatch::charged_bytes(Bytes::from(data), credit),
                observation: NativeChatMemberFrameObservation {
                    carrier,
                    original: self.original.clone(),
                },
            });
            Some(())
        })();
        if delivered.is_none() {
            self.loss.send_modify(|loss| *loss = loss.wrapping_add(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{keypair_from_seed, MeshService, NativeTcpPreparation};
    use catcoms_rt::{MeshTransport, PublishOnceError, PublishSubmission};
    use std::time::Duration;

    #[tokio::test]
    async fn native_member_gossip_uses_one_original_topic_and_revokes_handed_off_frames() {
        let mut preparation = NativeTcpPreparation::new((), Arc::new(()));
        let (receiver, id, _) = preparation
            .build(
                keypair_from_seed([118; 32]).unwrap(),
                &["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                &[],
            )
            .unwrap();
        let (sender, _) = MeshService::new_tcp(None, &[]).unwrap();
        assert!(sender.native_chat_member_ingress().is_none());
        let topic = Topic::new(vec![98; 32]);
        let (retire, signal) = watch::channel(false);
        let scope = RequestCancellation::new(signal, None);
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut original = receiver
                .native_chat_member_ingress()
                .unwrap()
                .open(topic.clone(), 64, scope.clone())
                .await
                .unwrap();
            assert!(matches!(
                receiver
                    .native_chat_member_ingress()
                    .unwrap()
                    .open(topic.clone(), 64, scope)
                    .await,
                Err(NativeChatMemberGossipError::Busy)
            ));
            let address = receiver.next_listen_addr().await.unwrap();
            sender
                .dial(format!("{address}/p2p/{id}").parse().unwrap())
                .await
                .unwrap();
            sender
                .wait_for_peer_connected(receiver.local_peer())
                .await
                .unwrap();
            let send = async |first: u16| {
                for ordinal in first..first + 200 {
                    let bytes = ordinal.to_be_bytes();
                    match sender
                        .publish_once(topic.clone(), Bytes::copy_from_slice(&bytes))
                        .await
                    {
                        Ok(PublishSubmission::Submitted) => return bytes,
                        Err(PublishOnceError::NoPeers) => {
                            tokio::time::sleep(Duration::from_millis(20)).await
                        }
                        other => panic!("unexpected gossip submission {other:?}"),
                    }
                }
                panic!("the native topic did not reach the peer");
            };
            let sent = send(0).await;
            let Some(NativeChatMemberGossipEvent::Frame(original_frame)) = original.next().await
            else {
                panic!("expected the original native member frame")
            };
            assert_eq!(
                original_frame.current_data(),
                Some((&topic, sender.local_peer(), sent.as_slice()))
            );
            retire.send_replace(true);
            assert!(original.next().await.is_none());
            assert!(original_frame.current_data().is_none());

            let (_owner, signal) = watch::channel(false);
            let scope = RequestCancellation::new(signal, None);
            let mut current = loop {
                match receiver
                    .native_chat_member_ingress()
                    .unwrap()
                    .open(topic.clone(), 64, scope.clone())
                    .await
                {
                    Ok(current) => break current,
                    Err(NativeChatMemberGossipError::Busy) => tokio::task::yield_now().await,
                    Err(error) => {
                        panic!("original native mesh refused a fresh subscription: {error}")
                    }
                }
            };
            let sent = send(200).await;
            let Some(NativeChatMemberGossipEvent::Frame(current_frame)) = current.next().await
            else {
                panic!("expected the fresh native member frame")
            };
            assert_eq!(
                current_frame.current_data(),
                Some((&topic, sender.local_peer(), sent.as_slice()))
            );
            assert!(original_frame.current_data().is_none());
            drop(current);
            assert!(current_frame.current_data().is_none());
        })
        .await
        .unwrap();
    }
}
