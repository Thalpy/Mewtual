//! Driver-acknowledged one-shot gossip. This module has no access to the legacy pending_publish
//! queue: retries belong to the caller, which can recheck authority and reseal a durable intent.
//! It does not remove libp2p's normal caches or retract messages once an attempt has started.

use std::sync::Arc;

use bytes::Bytes;
use catcoms_rt::{
    PublishOnceError, PublishSubmission, Topic, MAX_PUBLISH_ONCE_BYTES,
    MAX_PUBLISH_ONCE_TOPIC_BYTES,
};
use libp2p::gossipsub;
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};

use super::{to_ident, Command};

/// At most 8 MiB of compact payloads plus 1 KiB of topics queued/being attempted per service.
/// This excludes already-admitted gossip-cache/handler state and caller-owned input allocations.
pub(super) const MAX_IN_FLIGHT: usize = 16;

pub(super) struct Publication {
    topic: Topic,
    data: Bytes,
    reply: oneshot::Sender<Result<PublishSubmission, PublishOnceError>>,
    _slot: OwnedSemaphorePermit,
}

impl Publication {
    pub(super) fn run(self, gossip: &mut gossipsub::Behaviour) {
        self.run_with(|topic, data| classify(gossip.publish(to_ident(&topic), data.to_vec())));
    }

    /// The last closed-receiver check is the local admission boundary. Everything after it is
    /// synchronous; dropping the waiter after this check cannot roll back gossip side effects.
    /// In particular libp2p 0.49 inserts its message cache BEFORE some NoPeers/AllQueuesFull errors.
    fn run_with(
        self,
        attempt: impl FnOnce(Topic, Bytes) -> Result<PublishSubmission, PublishOnceError>,
    ) {
        if !self.reply.is_closed() {
            let result = attempt(self.topic, self.data);
            // A lost acknowledgement is ambiguous, never a reason to requeue raw ciphertext.
            let _ = self.reply.send(result);
        }
    }
}

fn classify(
    result: Result<gossipsub::MessageId, gossipsub::PublishError>,
) -> Result<PublishSubmission, PublishOnceError> {
    match result {
        Ok(_) => Ok(PublishSubmission::Submitted),
        Err(gossipsub::PublishError::Duplicate) => Ok(PublishSubmission::Duplicate),
        Err(gossipsub::PublishError::NoPeersSubscribedToTopic) => Err(PublishOnceError::NoPeers),
        Err(gossipsub::PublishError::AllQueuesFull(_)) => Err(PublishOnceError::QueuesFull),
        Err(gossipsub::PublishError::MessageTooLarge) => Err(PublishOnceError::TooLarge),
        Err(
            gossipsub::PublishError::SigningError(_) | gossipsub::PublishError::TransformFailed(_),
        ) => Err(PublishOnceError::Failed),
    }
}

pub(super) async fn publish_once(
    commands: &mpsc::Sender<Command>,
    slots: &Arc<Semaphore>,
    topic: Topic,
    data: Bytes,
) -> Result<PublishSubmission, PublishOnceError> {
    if data.len() > MAX_PUBLISH_ONCE_BYTES || topic.as_bytes().len() > MAX_PUBLISH_ONCE_TOPIC_BYTES
    {
        return Err(PublishOnceError::TooLarge);
    }
    let slot = slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| PublishOnceError::Busy)?;
    let (reply, result) = oneshot::channel();
    // Bytes/Topic can be slices of much larger allocations: compact only AFTER owning capacity.
    let publication = Publication {
        topic: Topic::new(Bytes::copy_from_slice(topic.as_bytes())),
        data: Bytes::copy_from_slice(&data),
        reply,
        _slot: slot,
    };
    drop(topic);
    drop(data);
    commands
        .try_send(Command::PublishOnce(publication))
        .map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => PublishOnceError::Busy,
            mpsc::error::TrySendError::Closed(_) => PublishOnceError::Closed,
        })?;
    // Drop of this receiver is the cancellation signal owned by the future itself. There is no
    // detached task which could keep the signal alive and later submit abandoned work.
    result.await.map_err(|_| PublishOnceError::Closed)?
}

#[cfg(test)]
mod tests;
