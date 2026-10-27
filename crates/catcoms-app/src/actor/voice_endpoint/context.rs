//! Bounded read-only native eligibility query. This is not call or action authority.
use super::*;

/// Original reply receiver for one admitted context snapshot. Dropping it does not
/// cancel the queued read, and neither admission nor successful receipt authorizes effects.
/// An abandoned read remains ordinary finite Actor mailbox/oneshot infrastructure: this
/// receiver does not claim native Control/SDP publication custody until Actor collection.
pub struct VoiceContextPending(oneshot::Receiver<Result<catcoms_sync::voice_v2::Context, String>>);
impl std::fmt::Debug for VoiceContextPending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceContextPending { .. }")
    }
}
impl VoiceContextPending {
    /// Read the actual Actor result, preserving the existing context-query error contract.
    pub async fn complete(self) -> Result<catcoms_sync::voice_v2::Context, String> {
        self.0
            .await
            .unwrap_or_else(|_| Err("server stopped".into()))
    }
}
impl ServerActor {
    /// Try to admit one read into the SAME finite mailbox before constructing its reply.
    /// A full mailbox is a proven local Capacity refusal, not an ambiguous handoff. Returning
    /// the thin receiver avoids retaining the largest AppCommand variant in a parked native
    /// preflight future. Final native/Root effect admission must still repeat all authority.
    pub fn try_queue_voice_v2_context(
        &self,
        room: u128,
    ) -> Result<VoiceContextPending, VoiceEndpointError> {
        let permit = self.cmd_tx.tx.try_reserve().map_err(|error| match error {
            mpsc::error::TrySendError::Full(()) => {
                VoiceEndpointError::Protocol(VoiceError::Capacity)
            }
            mpsc::error::TrySendError::Closed(()) => VoiceEndpointError::Stopped,
        })?;
        let (reply, receiver) = oneshot::channel();
        permit.send(Envelope {
            trace: self.cmd_tx.trace,
            command: AppCommand::VoiceV2Context { room, reply },
        });
        Ok(VoiceContextPending(receiver))
    }
}
/// Conservative fixed query-admission stack/queue/reply inventory for native Control proofs.
/// The single Envelope is moved to the existing bounded mailbox; the oneshot and reply
/// survive receiver loss until the actual queued read retires under existing bounded Actor
/// infrastructure. This inventory is not a transferable payer or a drainage witness.
/// No variable input is copied.
pub fn voice_context_queue_bytes() -> usize {
    std::mem::size_of::<Envelope>()
        + std::mem::size_of::<VoiceContextPending>()
        + std::mem::size_of::<Result<catcoms_sync::voice_v2::Context, String>>()
        + 512 // fixed oneshot/queue bookkeeping, permit, arguments, error string and alignment
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn full_mailbox_refuses_before_query_then_same_read_succeeds_after_capacity_returns() {
        let (actor, mut events, task, _, _) = super::super::tests::start().await;
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        let room = crate::channel_id("general");
        let expected = actor.voice_v2_context(room).await.unwrap();
        let paused = super::super::tests::park(&actor).await;
        let mut permits = Vec::new();
        while let Ok(permit) = actor.cmd_tx.tx.try_reserve() {
            permits.push(permit);
        }
        assert!(!permits.is_empty());
        assert_eq!(
            actor.try_queue_voice_v2_context(room).unwrap_err(),
            VoiceEndpointError::Protocol(VoiceError::Capacity)
        );
        drop(permits);
        let pending = actor.try_queue_voice_v2_context(room).unwrap();
        drop(paused);
        let observed = pending.complete().await.unwrap();
        assert_eq!(observed.scope, expected.scope);
        let paused = super::super::tests::park(&actor).await;
        drop(actor.try_queue_voice_v2_context(room).unwrap());
        drop(paused);
        actor.member_count().await; // actual cancelled receiver's query has retired
        assert_eq!(
            actor
                .try_queue_voice_v2_context(room)
                .unwrap()
                .complete()
                .await
                .unwrap()
                .scope,
            expected.scope
        );
        actor.shutdown().await;
        task.await.unwrap();
        drain.await.unwrap();
        assert_eq!(
            actor.try_queue_voice_v2_context(room).unwrap_err(),
            VoiceEndpointError::Stopped
        );
    }
    #[tokio::test]
    async fn admitted_read_reports_actual_actor_loss_without_requeueing() {
        let (actor, mut events, task, _, _) = super::super::tests::start().await;
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        let paused = super::super::tests::park(&actor).await;
        let pending = actor
            .try_queue_voice_v2_context(crate::channel_id("general"))
            .unwrap();
        // Positive queue return precedes this actual Actor loss. No sender, reconstructed
        // query or synthetic literal result replaces its original abandoned reply.
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(paused);
        assert_eq!(pending.complete().await.unwrap_err(), "server stopped");
        assert_eq!(
            actor
                .try_queue_voice_v2_context(crate::channel_id("general"))
                .unwrap_err(),
            VoiceEndpointError::Stopped
        );
        drain.await.unwrap();
    }
}
