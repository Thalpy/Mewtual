//! Bind one received session to the exact native founder runtime and mesh.
catcoms_rt::redacted_debug! {
    [R: CryptoRngCore] NativeTextFounderMemberIngressPreparation<R>;
    [R: CryptoRngCore] NativeTextFounderMemberReceiver<R>;
    [R: CryptoRngCore] NativeTextFounderDependencyProvider<R>;
}

use super::super::super::governed::{
    NativeTextFounderReviewExchange, NativeTextFounderReviewReply, NativeTextFounderReviewResult,
    NativeTextFounderReviewSweep,
};
use super::*;
use crate::store::persistence::v2_member_transport::{
    FounderReceiveOperation, FounderReceiveOutcome, FounderReceiveSession,
    NativeTextFounderDependencyResponse, NativeTextFounderFrameOutcome, NativeTextFounderReceived,
};
use crate::store::persistence::v2_shared_history::SharedHistoryActionQuery;
use crate::store::{NativeTextFounderAudit, NativeTextJoinedArchivedActions};
use catcoms_net::{
    NativeChatMemberDependencyIncoming, NativeChatMemberDependencyRequests, NativeChatMemberFrame,
    NativeChatMemberGossip, NativeChatMemberGossipError, NativeChatMemberGossipEvent,
    NativeChatMemberIngress,
};
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::ActionId;
use catcoms_rt::{RequestAuthority, Topic};
use catcoms_sync::owner_v2::{V2RemovedReviewQuery, MAX_V2_MEMBER_TRANSPORT_BYTES};
use catcoms_sync::V2_MEMBER_CONTACT_BYTES;
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

#[derive(Debug)]
struct DependencyScope(
    RequestCancellation,
    Option<catcoms_replication::chat::V2LiveMemberObservation>,
);

#[async_trait::async_trait]
impl RequestAuthority for DependencyScope {
    fn authorized(&self) -> bool {
        !self.0.is_cancelled()
            && self
                .1
                .as_ref()
                .is_none_or(|requester| requester.is_live_cached())
    }

    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        // The original founder publication observation also retires on
        // selected epoch changes. The requester test repeats at delivery.
        self.0.clone().cancelled().await;
    }
}

pub struct NativeTextFounderDependencyProvider<R: CryptoRngCore> {
    _owner: watch::Sender<bool>,
    row: NativeTextRuntimeSelection<catcoms_net::MeshService, R>,
    channel: u128,
    signal: RequestCancellation,
    requests: NativeChatMemberDependencyRequests,
}

impl<R: CryptoRngCore> NativeTextFounderDependencyProvider<R> {
    pub async fn next(&mut self) -> Option<NativeChatMemberDependencyIncoming> {
        self.requests.next().await
    }
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
    pub fn attach_founder_dependency_provider(
        self,
        channel: u128,
        caller: RequestCancellation,
    ) -> Result<NativeTextFounderDependencyProvider<R>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let row = self.row.clone();
        let lane = row
            .original
            .dependency_lane
            .as_ref()
            .ok_or(Refusal::Unavailable)?;
        let (cell, mut keys) = self.prepare_keys()?;
        let source = keys.member_entry().1.observe_current();
        let (owner, live) = watch::channel(false);
        let caller =
            connected::constrain_founder_transport(self.scope.clone(), source, caller, Some(live));
        let FounderReceiveOutcome::Subscription(_, observation, fee) = self
            .finish_prepared((cell, keys))?
            .run_founder_member_receive(channel, FounderReceiveOperation::Subscription, &caller)?
        else {
            return Err(Refusal::Failed);
        };
        let signal = observation
            .constrain(caller)
            .retain_custody(Arc::new(IngressPayment { _fee: fee }));
        let requests = lane
            .try_attach(Arc::new(DependencyScope(signal.clone(), None)))
            .ok_or(Refusal::Busy)?;
        Ok(NativeTextFounderDependencyProvider {
            _owner: owner,
            row,
            channel,
            signal,
            requests,
        })
    }

    pub fn serve_founder_dependency_request(
        self,
        provider: &NativeTextFounderDependencyProvider<R>,
        incoming: NativeChatMemberDependencyIncoming,
    ) -> Result<(), V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !self.row.same_original(&provider.row)
            || !self
                .row
                .original
                .dependency_lane
                .as_ref()
                .is_some_and(|lane| lane.owns_request(&incoming))
        {
            return Err(Refusal::Stale);
        }
        let FounderReceiveOutcome::Dependency(reply) = self.finish()?.run_founder_member_receive(
            provider.channel,
            FounderReceiveOperation::Dependency(&incoming),
            &provider.signal,
        )?
        else {
            return Err(Refusal::Failed);
        };
        let signal = reply.observation.constrain(provider.signal.clone());
        incoming
            .respond(
                reply.wire,
                Arc::new(DependencyScope(signal, reply.requester)),
                Arc::new(IngressPayment { _fee: reply.fee }),
            )
            .map_err(|_| Refusal::Cancelled)
    }

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
        admit_mutation: &dyn crate::store::FounderArchiveAdmission,
    ) -> Result<Option<NativeTextFounderReceived>, V2MemberSaveRefusal> {
        Ok(self
            .receive_founder_frame(receiver, frame, false, admit_mutation)?
            .retained())
    }

    pub fn receive_founder_member_with_recovery(
        self,
        receiver: &NativeTextFounderMemberReceiver<R>,
        frame: &NativeChatMemberFrame,
        admit_mutation: &dyn crate::store::FounderArchiveAdmission,
    ) -> Result<NativeTextFounderFrameOutcome, V2MemberSaveRefusal> {
        self.receive_founder_frame(receiver, frame, true, admit_mutation)
    }

    fn receive_founder_frame(
        self,
        receiver: &NativeTextFounderMemberReceiver<R>,
        frame: &NativeChatMemberFrame,
        recovery: bool,
        admit_mutation: &dyn crate::store::FounderArchiveAdmission,
    ) -> Result<NativeTextFounderFrameOutcome, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !self.row.same_original(&receiver.row) || !receiver.ingress.owns_frame(frame) {
            return Err(Refusal::Stale);
        }
        let carrier = if recovery {
            self.row
                .original
                .dependency_lane
                .as_ref()
                .and_then(|lane| lane.capture_frame_request(&frame.observe()))
        } else {
            None
        };
        match self.finish()?.run_founder_member_receive(
            receiver.channel,
            FounderReceiveOperation::Frame(frame, &receiver.session, carrier, admit_mutation),
            &receiver.session.signal,
        )? {
            FounderReceiveOutcome::Frame(action) => Ok(action),
            _ => Err(Refusal::Failed),
        }
    }

    pub fn recover_founder_dependency(
        self,
        receiver: &NativeTextFounderMemberReceiver<R>,
        response: &NativeTextFounderDependencyResponse,
        admit_mutation: &dyn crate::store::FounderArchiveAdmission,
    ) -> Result<bool, V2MemberSaveRefusal> {
        Ok(self
            .recover_founder_dependency_with_followup(receiver, response, false, admit_mutation)?
            .retained()
            .is_some())
    }

    pub fn recover_founder_dependency_with_followup(
        self,
        receiver: &NativeTextFounderMemberReceiver<R>,
        response: &NativeTextFounderDependencyResponse,
        followup: bool,
        admit_mutation: &dyn crate::store::FounderArchiveAdmission,
    ) -> Result<NativeTextFounderFrameOutcome, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !self.row.same_original(&receiver.row) {
            return Err(Refusal::Stale);
        }
        response.require_current(&receiver.session)?;
        let carrier = if followup {
            self.row
                .original
                .dependency_lane
                .as_ref()
                .and_then(|lane| response.capture_followup(lane))
        } else {
            None
        };
        match self.finish()?.run_founder_member_receive(
            receiver.channel,
            FounderReceiveOperation::Recover(response, &receiver.session, carrier, admit_mutation),
            &receiver.session.signal,
        )? {
            FounderReceiveOutcome::Recovered(retained) => Ok(retained),
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
            FounderReceiveOutcome::RecoveredProjection(projected) => Ok(projected),
            _ => Err(V2MemberSaveRefusal::Failed),
        }
    }

    pub fn archived_founder_action_page(
        self,
        channel: u128,
        after: Option<ActionId>,
        limit: usize,
        caller: &RequestCancellation,
    ) -> Result<Option<NativeTextJoinedArchivedActions>, V2MemberSaveRefusal> {
        let query = SharedHistoryActionQuery {
            target: None,
            after,
            limit,
        };
        match self.finish()?.run_founder_member_receive(
            channel,
            FounderReceiveOperation::ArchivePage(query),
            caller,
        )? {
            FounderReceiveOutcome::ArchivePage(page) => {
                Ok(page.map(NativeTextJoinedArchivedActions))
            }
            _ => Err(V2MemberSaveRefusal::Failed),
        }
    }

    pub fn archived_founder_message_projection(
        self,
        channel: u128,
        action: ActionId,
        caller: &RequestCancellation,
    ) -> Result<Option<NativeTextJoinedMemberProjection>, V2MemberSaveRefusal> {
        self.founder_target_projection(
            channel,
            FounderReceiveOperation::ArchiveMessage(action),
            caller,
        )
    }

    /// Refresh one received message. The selected archive takes precedence
    /// over a live revision, even when its selected effect is Delete.
    pub fn read_founder_target_projection(
        self,
        channel: u128,
        action: ActionId,
        caller: &RequestCancellation,
    ) -> Result<Option<NativeTextJoinedMemberProjection>, V2MemberSaveRefusal> {
        self.founder_target_projection(
            channel,
            FounderReceiveOperation::TargetMessage(action),
            caller,
        )
    }

    fn founder_target_projection(
        self,
        channel: u128,
        operation: FounderReceiveOperation<'_>,
        caller: &RequestCancellation,
    ) -> Result<Option<NativeTextJoinedMemberProjection>, V2MemberSaveRefusal> {
        let (cell, mut keys) = self.prepare_keys()?;
        let source = keys.member_entry().1.observe_current();
        let caller = connected::constrain_founder_transport(
            self.scope.clone(),
            source,
            caller.clone(),
            None,
        );
        match self
            .finish_prepared((cell, keys))?
            .run_founder_member_receive(channel, operation, &caller)?
        {
            FounderReceiveOutcome::ArchiveMessage(result) => Ok(result.map(|original| {
                NativeTextJoinedMemberProjection::from_archive(
                    original.projection,
                    original.signal,
                    original.fee,
                    original.lease,
                )
            })),
            FounderReceiveOutcome::Read(projected) => {
                Ok(projected.map(|(projection, signal, fee)| {
                    NativeTextJoinedMemberProjection::from_live(projection, signal, fee)
                }))
            }
            FounderReceiveOutcome::RecoveredProjection(projected) => Ok(projected),
            _ => Err(V2MemberSaveRefusal::Failed),
        }
    }

    /// Resolve an exact archive reservation under the current founder row,
    /// independent of the original peer's connection or UI projection.
    pub fn selected_founder_archive_target(
        self,
        channel: u128,
        action: ActionId,
        caller: &RequestCancellation,
    ) -> Result<Option<ActionId>, V2MemberSaveRefusal> {
        match self.finish()?.run_founder_member_receive(
            channel,
            FounderReceiveOperation::SelectedTarget(action),
            caller,
        )? {
            FounderReceiveOutcome::SelectedTarget(target) => Ok(target),
            _ => Err(V2MemberSaveRefusal::Failed),
        }
    }

    /// Read selected signed Creates, or the exact target's retained originals,
    /// under the live founder source. This cannot settle archive writes.
    pub fn audit_founder_history(
        self,
        channel: u128,
        target: Option<ActionId>,
        after: Option<ActionId>,
        limit: usize,
        caller: &RequestCancellation,
    ) -> Result<NativeTextFounderAudit, V2MemberSaveRefusal> {
        let (cell, mut keys) = self.prepare_keys()?;
        let source = keys.member_entry().1.observe_current();
        let caller = connected::constrain_founder_transport(
            self.scope.clone(),
            source,
            caller.clone(),
            None,
        );
        let query = SharedHistoryActionQuery {
            target,
            after,
            limit,
        };
        match self
            .finish_prepared((cell, keys))?
            .run_founder_member_receive(channel, FounderReceiveOperation::Audit(query), &caller)?
        {
            FounderReceiveOutcome::Audit(audit) => Ok(audit),
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
        let rng = body.rng.as_mut().ok_or(Refusal::Unavailable)?;
        let result = if let Some(founder) = body.recovered_founder.as_mut() {
            run_recovered_receive(founder, channel, operation, caller, rng).map_err(|failure| {
                match failure {
                    AppError::ChatV2ArchiveBusy => Refusal::Busy,
                    AppError::ChatV2ReceiveCapacity => Refusal::ReceiveCapacity,
                    _ => Refusal::Failed,
                }
            })
        } else {
            body.work.run_founder_member_receive(
                body.driver.as_mut().ok_or(Refusal::Unavailable)?,
                channel,
                operation,
                caller,
                rng,
                &mut inner.keys,
            )
        };
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

impl<R: CryptoRngCore> NativeTextMemberPreparation<catcoms_net::MeshService, R> {
    /// Capture a review sweep on the current founder row and an explicitly
    /// approved signed contact. This only borrows an existing connection.
    pub fn start_founder_removed_review(
        self,
        channel: u128,
        admission: OwnerFactId,
        contact: [u8; V2_MEMBER_CONTACT_BYTES],
        caller: &RequestCancellation,
    ) -> Result<Arc<NativeTextFounderReviewSweep>, V2MemberSaveRefusal> {
        self.with_recovered_review(caller, |founder, signal, _| {
            founder.start_removed_review(channel, admission, contact, signal)
        })
    }

    /// Release the original restored runtime before awaiting the exact
    /// inventory or fetch request. Only its negative and payment survive.
    pub fn prepare_founder_removed_review(
        self,
        sweep: Arc<NativeTextFounderReviewSweep>,
        query: V2RemovedReviewQuery,
        caller: &RequestCancellation,
    ) -> Result<NativeTextFounderReviewExchange, V2MemberSaveRefusal> {
        self.with_recovered_review(caller, |founder, signal, rng| {
            founder.prepare_removed_review(sweep, query, signal, rng)
        })
    }

    /// Reacquire the original selected row before authenticating a received
    /// review reply. Exact fetches can only produce a local hold receipt.
    pub fn receive_founder_removed_review(
        self,
        reply: NativeTextFounderReviewReply,
        caller: &RequestCancellation,
    ) -> Result<NativeTextFounderReviewResult, V2MemberSaveRefusal> {
        self.with_recovered_review(caller, |founder, signal, rng| {
            founder.receive_removed_review(reply, signal, rng)
        })
    }

    fn with_recovered_review<T>(
        self,
        caller: &RequestCancellation,
        work: impl FnOnce(
            &mut crate::store::NativeTextColdFounderSource,
            &RequestCancellation,
            &mut R,
        ) -> Result<T, AppError>,
    ) -> Result<T, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let (cell, mut keys) = self.prepare_keys()?;
        let source = keys.member_entry().1.observe_current();
        let signal = connected::constrain_founder_transport(
            self.scope.clone(),
            source,
            caller.clone(),
            None,
        );
        let mut job = self.finish_prepared((cell, keys))?;
        let inner = &mut job.inner;
        inner.require_current().map_err(|_| Refusal::Cancelled)?;
        let body = inner.loan.body.as_mut().ok_or(Refusal::Unavailable)?;
        let founder = body
            .recovered_founder
            .as_mut()
            .ok_or(Refusal::Unavailable)?;
        let rng = body.rng.as_mut().ok_or(Refusal::Unavailable)?;
        let result = work(founder, &signal, rng);
        inner.loan.recorded = true;
        inner.require_current().map_err(|_| Refusal::Cancelled)?;
        if signal.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        result.map_err(|error| match error {
            AppError::ChatV2ArchiveBusy => Refusal::Busy,
            _ => Refusal::Failed,
        })
    }
}

fn run_recovered_receive(
    founder: &mut crate::store::NativeTextColdFounderSource,
    channel: u128,
    operation: FounderReceiveOperation<'_>,
    caller: &RequestCancellation,
    rng: &mut impl CryptoRngCore,
) -> Result<FounderReceiveOutcome, AppError> {
    Ok(match operation {
        FounderReceiveOperation::Subscription => {
            let (topic, observation, fee) = founder.prepare_member_subscription(channel, caller)?;
            FounderReceiveOutcome::Subscription(topic, observation, fee)
        }
        FounderReceiveOperation::Frame(frame, session, carrier, admit_mutation) => {
            FounderReceiveOutcome::Frame(founder.receive_selected_member_frame(
                channel,
                session,
                frame,
                (carrier, admit_mutation),
                rng,
            )?)
        }
        FounderReceiveOperation::Recover(response, session, carrier, admit_mutation) => {
            FounderReceiveOutcome::Recovered(founder.recover_selected_dependency(
                channel,
                session,
                response,
                (carrier, admit_mutation),
                rng,
            )?)
        }
        FounderReceiveOperation::Dependency(incoming) => FounderReceiveOutcome::Dependency(
            founder.prepare_dependency_reply(channel, incoming, caller, rng)?,
        ),
        FounderReceiveOperation::Read(action) => FounderReceiveOutcome::RecoveredProjection(
            founder.read_member_projection(channel, action, caller)?,
        ),
        FounderReceiveOperation::ArchivePage(query) => {
            let page = founder.archived_action_page(channel, query.after, query.limit, caller)?;
            FounderReceiveOutcome::ArchivePage(page.map(|page| page.0))
        }
        FounderReceiveOperation::ArchiveMessage(action) => {
            FounderReceiveOutcome::RecoveredProjection(
                founder.archived_message_projection(channel, action, caller, rng)?,
            )
        }
        FounderReceiveOperation::TargetMessage(action) => {
            let (mounted, archived) =
                founder.received_archive_lookup(channel, action, caller, rng)?;
            FounderReceiveOutcome::RecoveredProjection(if mounted {
                archived
            } else {
                founder.read_member_projection(channel, action, caller)?
            })
        }
        FounderReceiveOperation::SelectedTarget(action) => FounderReceiveOutcome::SelectedTarget(
            founder.selected_archive_target(channel, action, caller)?,
        ),
        FounderReceiveOperation::Audit(query) => {
            FounderReceiveOutcome::Audit(founder.selected_history_audit(channel, query, caller)?)
        }
    })
}
