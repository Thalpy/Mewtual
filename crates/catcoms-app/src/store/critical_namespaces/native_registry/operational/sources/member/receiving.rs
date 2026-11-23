//! Bind one received session to the exact native founder runtime and mesh.
use super::*;
use crate::store::persistence::v2_member_transport::{
    FounderReceiveOperation, FounderReceiveOutcome, FounderReceiveSession,
};
use catcoms_net::{
    NativeChatMemberFrame, NativeChatMemberGossip, NativeChatMemberGossipError,
    NativeChatMemberGossipEvent, NativeChatMemberIngress,
};
use catcoms_replication::chat::ActionId;
use catcoms_rt::Topic;
use catcoms_sync::owner_v2::MAX_V2_MEMBER_TRANSPORT_BYTES;
use tokio::sync::watch;

struct IngressPayment {
    _fee: CatalogReadReservation,
}

impl std::fmt::Debug for IngressPayment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FounderIngressPayment")
            .finish_non_exhaustive()
    }
}

pub struct NativeTextFounderMemberIngressPreparation<R: CryptoRngCore> {
    channel: u128,
    topic: Topic,
    ingress: NativeChatMemberIngress,
    row: NativeTextRuntimeSelection<catcoms_net::MeshService, R>,
    session: FounderReceiveSession,
    owner: watch::Sender<bool>,
    _fee: Arc<IngressPayment>,
}

pub struct NativeTextFounderMemberReceiver<R: CryptoRngCore> {
    _owner: watch::Sender<bool>,
    receiver: NativeChatMemberGossip,
    channel: u128,
    ingress: NativeChatMemberIngress,
    row: NativeTextRuntimeSelection<catcoms_net::MeshService, R>,
    session: FounderReceiveSession,
}

impl<R: CryptoRngCore> NativeTextFounderMemberIngressPreparation<R> {
    pub async fn open(
        self,
    ) -> Result<NativeTextFounderMemberReceiver<R>, NativeChatMemberGossipError> {
        let signal = self.session.signal.clone().retain_custody(self._fee);
        let receiver = self
            .ingress
            .clone()
            .open(self.topic, MAX_V2_MEMBER_TRANSPORT_BYTES, signal)
            .await?;
        Ok(NativeTextFounderMemberReceiver {
            _owner: self.owner,
            receiver,
            channel: self.channel,
            ingress: self.ingress,
            row: self.row,
            session: self.session,
        })
    }
}

impl<R: CryptoRngCore> NativeTextFounderMemberReceiver<R> {
    pub async fn next(&mut self) -> Option<NativeChatMemberGossipEvent> {
        self.receiver.next().await
    }
}

impl<R: CryptoRngCore> NativeTextMemberPreparation<catcoms_net::MeshService, R> {
    pub fn prepare_founder_member_ingress(
        self,
        channel: u128,
        caller: RequestCancellation,
    ) -> Result<NativeTextFounderMemberIngressPreparation<R>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let ingress = self
            .row
            .original
            .member_ingress
            .clone()
            .ok_or(Refusal::Unavailable)?;
        let row = self.row.clone();
        let (cell, mut keys) = self.prepare_keys()?;
        let source = keys.member_entry().1.observe_current();
        let (owner, live) = watch::channel(false);
        let caller =
            connected::constrain_founder_transport(self.scope.clone(), source, caller, Some(live));
        let result = self
            .finish_prepared((cell, keys))?
            .run_founder_member_receive(channel, FounderReceiveOperation::Subscription, &caller)?;
        let FounderReceiveOutcome::Subscription(topic, observation, fee) = result else {
            return Err(Refusal::Failed);
        };
        let signal = observation.constrain(caller);
        if signal.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        Ok(NativeTextFounderMemberIngressPreparation {
            channel,
            topic,
            ingress,
            row,
            owner,
            session: FounderReceiveSession {
                identity: Arc::new(()),
                signal,
            },
            _fee: Arc::new(IngressPayment { _fee: fee }),
        })
    }

    pub fn receive_founder_member(
        self,
        receiver: &NativeTextFounderMemberReceiver<R>,
        frame: &NativeChatMemberFrame,
    ) -> Result<Option<ActionId>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !self.row.same_original(&receiver.row) || !receiver.ingress.owns_frame(frame) {
            return Err(Refusal::Stale);
        }
        match self.finish()?.run_founder_member_receive(
            receiver.channel,
            FounderReceiveOperation::Frame(frame, &receiver.session),
            &receiver.session.signal,
        )? {
            FounderReceiveOutcome::Frame(action) => Ok(action),
            _ => Err(Refusal::Failed),
        }
    }

    pub fn read_founder_member_projection(
        self,
        channel: u128,
        action: ActionId,
        caller: &RequestCancellation,
    ) -> Result<Option<NativeTextJoinedMemberProjection>, V2MemberSaveRefusal> {
        match self.finish()?.run_founder_member_receive(
            channel,
            FounderReceiveOperation::Read(action),
            caller,
        )? {
            FounderReceiveOutcome::Read(projected) => {
                Ok(projected.map(|(projection, signal, fee)| {
                    NativeTextJoinedMemberProjection::from_live(projection, signal, fee)
                }))
            }
            _ => Err(V2MemberSaveRefusal::Failed),
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextMemberJob<T, R> {
    fn run_founder_member_receive(
        mut self,
        channel: u128,
        operation: FounderReceiveOperation<'_>,
        caller: &RequestCancellation,
    ) -> Result<FounderReceiveOutcome, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        self.inner
            .require_current()
            .map_err(|_| Refusal::Unavailable)?;
        let inner = &mut self.inner;
        let body = inner.loan.body.as_mut().ok_or(Refusal::Unavailable)?;
        let result = body.work.run_founder_member_receive(
            body.driver.as_mut().ok_or(Refusal::Unavailable)?,
            channel,
            operation,
            caller,
            &mut inner.keys,
        );
        inner.loan.recorded = true;
        self.inner
            .require_current()
            .map_err(|_| Refusal::Cancelled)?;
        if caller.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        result
    }
}
