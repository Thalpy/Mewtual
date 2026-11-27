//! Live ordinary text from the recovered founder's exact reserved mesh.
catcoms_rt::redacted_debug! {
    NativeTextColdFounderMemberIngress;
    NativeTextColdFounderMemberReceiver;
}

use super::*;
use crate::store::persistence::native_registry::network::NetworkDependency;
use crate::store::persistence::v2_member_transport::{
    archive_founder_member, first_founder_missing, prepare_founder_dependency_exchange,
    prepare_founder_dependency_reply, project_founder_member, FounderDependencyReply,
    FounderReceiveSession, NativeTextFounderDependencyResponse, NativeTextFounderFrameOutcome,
    FOUNDER_DEPENDENCY_REPLY_FEE, FOUNDER_RECEIVE_ACTIONS, FOUNDER_RECEIVE_BYTES,
    FOUNDER_RECEIVE_LEDGER_FEE, FOUNDER_RECEIVE_WORK,
};
use crate::store::{
    NativeTextFounderReceived, NativeTextJoinedMemberProjection, V2MemberSaveRefusal,
};
use catcoms_net::{
    NativeChatMemberDependencyExchange, NativeChatMemberDependencyIncoming, NativeChatMemberFrame,
    NativeChatMemberGossip, NativeChatMemberGossipError, NativeChatMemberGossipEvent,
    NativeChatMemberIngress,
};
use catcoms_replication::chat::{
    ActionId, ActionState, ChatLedger, LedgerLimits, ProjectionView, Realm,
    V2OrdinaryReceiveStatus, MAX_ACTION_BYTES,
};
use catcoms_rt::Topic;
use catcoms_sync::owner_v2::{V2MemberPublicationObservation, MAX_V2_MEMBER_TRANSPORT_BYTES};

const INGRESS_FEE: usize = 2 * MAX_V2_MEMBER_TRANSPORT_BYTES + 8192;
const FRAME_FEE: usize = 5 * MAX_V2_MEMBER_TRANSPORT_BYTES + 4096;

#[derive(Debug)]
struct ProjectionScope {
    receiver: RequestCancellation,
    caller: RequestCancellation,
}

#[async_trait::async_trait]
impl RequestAuthority for ProjectionScope {
    fn authorized(&self) -> bool {
        !self.receiver.is_cancelled() && !self.caller.is_cancelled()
    }

    async fn revoked(&self) {
        let (mut receiver, mut caller) = (self.receiver.clone(), self.caller.clone());
        tokio::select! {
            _ = receiver.cancelled() => {},
            _ = caller.cancelled() => {},
        }
    }
}

pub struct NativeTextColdFounderMemberIngress {
    ingress: NativeChatMemberIngress,
    original: Arc<NetworkDependency>,
    topic: Topic,
    channel: u128,
    owner: watch::Sender<()>,
    session: FounderReceiveSession,
}

pub struct NativeTextColdFounderMemberReceiver {
    _owner: watch::Sender<()>,
    receiver: NativeChatMemberGossip,
    ingress: NativeChatMemberIngress,
    original: Arc<NetworkDependency>,
    channel: u128,
    session: FounderReceiveSession,
}

impl NativeTextColdFounderMemberIngress {
    pub async fn open(
        self,
    ) -> Result<NativeTextColdFounderMemberReceiver, NativeChatMemberGossipError> {
        let receiver = self
            .ingress
            .clone()
            .open(
                self.topic,
                MAX_V2_MEMBER_TRANSPORT_BYTES,
                self.session.signal.clone(),
            )
            .await?;
        Ok(NativeTextColdFounderMemberReceiver {
            _owner: self.owner,
            receiver,
            ingress: self.ingress,
            original: self.original,
            channel: self.channel,
            session: self.session,
        })
    }
}

impl NativeTextColdFounderMemberReceiver {
    pub async fn next(&mut self) -> Option<NativeChatMemberGossipEvent> {
        self.receiver.next().await
    }
}

pub(super) struct Ledger {
    view: ProjectionView,
    ledger: ChatLedger,
    session: FounderReceiveSession,
    _fee: CatalogReadReservation,
}

impl NativeTextColdFounderSource {
    pub(in crate::store::critical_namespaces::native_registry) fn prepare_dependency_reply(
        &mut self,
        channel: u128,
        incoming: &NativeChatMemberDependencyIncoming,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<FounderDependencyReply, AppError> {
        if !self.runtime_dependency_lane()?.owns_request(incoming) {
            return Err(refused());
        }
        self.restore_first_add(caller)?;
        let scope = self.history_scope(channel)?;
        let (mount, fee) = self
            .work
            .recovered_founder_resources(FOUNDER_DEPENDENCY_REPLY_FEE)?;
        let (verifier, _) = self.governed.as_ref().ok_or_else(refused)?.member_parts()?;
        let provider = self
            .sdk
            .as_ref()
            .ok_or_else(refused)?
            .select_member_dependency_access(verifier, channel, self.local.0, mount)?;
        let reply =
            self.work
                .with_recovered_founder_store(&self.selection, &self.source, |store| {
                    let context = history::outbox_context(&self.selection, &self.source);
                    prepare_founder_dependency_reply(
                        provider,
                        incoming,
                        fee,
                        caller,
                        rng,
                        |id, rng| {
                            history::saved_dependency(store, context, &self.source, scope, id, rng)
                        },
                    )
                    .map_err(|_| refused())
                })?;
        self.restore_first_add(caller)?;
        Ok(reply)
    }

    pub fn prepare_member_ingress(
        &mut self,
        channel: u128,
        caller: RequestCancellation,
    ) -> Result<NativeTextColdFounderMemberIngress, AppError> {
        let mesh = self.actor.as_ref().ok_or_else(refused)?.borrow()?;
        let ingress = mesh.native_chat_member_ingress().ok_or_else(refused)?;
        let (topic, observation, fee) = self.prepare_member_subscription(channel, &caller)?;
        let (mount, _) = self.work.recovered_founder_resources(4096)?;
        let caller = observation.constrain(caller);
        let (owner, lifetime) = watch::channel(());
        let scope = Arc::new(IngressScope {
            lifetime,
            mount,
            caller,
        });
        let signal = scope
            .caller
            .clone()
            .with_authority(scope.clone())
            .retain_custody(Arc::new(JoinFramePayment { _fee: fee }));
        self.restore_first_add(&signal)?;
        Ok(NativeTextColdFounderMemberIngress {
            ingress,
            original: self.selection.network.clone(),
            topic,
            channel,
            owner,
            session: FounderReceiveSession {
                identity: Arc::new(()),
                signal,
            },
        })
    }

    pub(in crate::store::critical_namespaces::native_registry) fn prepare_member_subscription(
        &mut self,
        channel: u128,
        caller: &RequestCancellation,
    ) -> Result<
        (
            Topic,
            V2MemberPublicationObservation,
            CatalogReadReservation,
        ),
        AppError,
    > {
        self.restore_first_add(caller)?;
        let (mount, fee) = self.work.recovered_founder_resources(INGRESS_FEE)?;
        let (verifier, _) = self.governed.as_ref().ok_or_else(refused)?.member_parts()?;
        let sdk = self.sdk.as_ref().ok_or_else(refused)?;
        let (topic, observation) = sdk.prepare_member_topic(verifier, channel, mount)?;
        self.restore_first_add(caller)?;
        Ok((topic, observation, fee))
    }

    pub fn receive_member_frame(
        &mut self,
        receiver: &NativeTextColdFounderMemberReceiver,
        frame: &NativeChatMemberFrame,
        admit_mutation: &dyn crate::store::FounderArchiveAdmission,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<NativeTextFounderReceived>, AppError> {
        if !Arc::ptr_eq(&receiver.original, &self.selection.network)
            || !receiver.ingress.owns_frame(frame)
        {
            return Err(refused());
        }
        Ok(self
            .receive_selected_member_frame(
                receiver.channel,
                &receiver.session,
                frame,
                (None, admit_mutation),
                rng,
            )?
            .retained())
    }

    pub(in crate::store::critical_namespaces::native_registry) fn receive_selected_member_frame(
        &mut self,
        channel: u128,
        session: &FounderReceiveSession,
        frame: &NativeChatMemberFrame,
        (carrier, admit_mutation): (
            Option<NativeChatMemberDependencyExchange>,
            &dyn crate::store::FounderArchiveAdmission,
        ),
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextFounderFrameOutcome, AppError> {
        let ingress = self.runtime_ingress()?.1;
        if !ingress.owns_frame(frame) {
            return Err(refused());
        }
        self.restore_first_add(&session.signal)?;
        let scope = self.history_scope(channel)?;
        let (verifier, group) = self.governed.as_ref().ok_or_else(refused)?.member_parts()?;
        let sdk = self.sdk.as_ref().ok_or_else(refused)?;
        let (mount, _payment) = self.work.recovered_founder_resources(FRAME_FEE)?;
        let (topic, _, wire) = frame.current_data().ok_or_else(refused)?;
        let original = sdk.open_member_envelope(verifier, channel, topic, wire, mount)?;
        if self
            .received
            .as_ref()
            .is_some_and(|ledger| ledger.session.signal.is_cancelled())
        {
            self.received = None;
        }
        if self.received.is_none() {
            let (_, fee) = self
                .work
                .recovered_founder_resources(FOUNDER_RECEIVE_LEDGER_FEE)?;
            let realm = Realm::new(&scope.group, channel).map_err(|_| refused())?;
            let mut ledger = ChatLedger::new(
                realm,
                LedgerLimits {
                    max_actions: FOUNDER_RECEIVE_ACTIONS,
                    max_action_bytes: FOUNDER_RECEIVE_BYTES,
                },
            )
            .map_err(|_| refused())?;
            let mut view = ledger
                .begin_projection_view_in(group)
                .map_err(|_| refused())?;
            if !view.try_ready() {
                return Err(refused());
            }
            self.received = Some(Ledger {
                view,
                ledger,
                session: session.clone(),
                _fee: fee,
            });
        }
        let selection = &self.selection;
        let source = &self.source;
        let work = &self.work;
        let ledger = self
            .received
            .as_mut()
            .filter(|ledger| {
                Arc::ptr_eq(&ledger.session.identity, &session.identity)
                    && ledger.ledger.realm().channel() == channel
            })
            .ok_or_else(refused)?;
        let check_lifetime = || {
            if session.signal.is_cancelled() || frame.current_data().is_none() {
                return Err(refused());
            }
            work.require_recovered_founder_source(selection, source)
        };
        let status = work.with_recovered_founder_store(selection, source, |store| {
            project_founder_member(
                &original,
                verifier,
                &mut ledger.ledger,
                &ledger.view,
                (store, scope, admit_mutation),
                |id| {
                    let context = history::outbox_context(selection, source);
                    history::saved_dependency(store, context, source, scope, id, rng)
                },
                || check_lifetime().map_err(|_| V2MemberSaveRefusal::Stale),
            )
            .map_err(|failure| match failure {
                V2MemberSaveRefusal::Busy => AppError::ChatV2ArchiveBusy,
                _ => refused(),
            })
        })?;
        if let (Some(carrier), Some(id)) = (carrier, first_founder_missing(&status)) {
            check_lifetime()?;
            let (mount, fee) = self
                .work
                .recovered_founder_resources(FOUNDER_DEPENDENCY_REPLY_FEE)?;
            let access =
                sdk.select_member_dependency_access(verifier, channel, self.local.0, mount)?;
            let request = prepare_founder_dependency_exchange(
                access,
                carrier,
                (session, None),
                &original,
                id,
                fee,
                rng,
            )
            .map_err(|_| refused())?;
            self.restore_first_add(&session.signal)?;
            return Ok(NativeTextFounderFrameOutcome::NeedsDependency(request));
        }
        if status != V2OrdinaryReceiveStatus::Retained(ActionState::Valid) {
            return Ok(NativeTextFounderFrameOutcome::Unresolved);
        }
        self.restore_first_add(&session.signal)?;
        let ledger = self
            .received
            .as_ref()
            .filter(|ledger| Arc::ptr_eq(&ledger.session.identity, &session.identity))
            .ok_or_else(refused)?;
        let governed = self.governed.as_mut().ok_or_else(refused)?;
        let archived = self
            .work
            .with_recovered_founder_store(&self.selection, &self.source, |store| {
                archive_founder_member(
                    (store, scope),
                    &original,
                    &ledger.ledger,
                    rng,
                    |dependency, rng| {
                        let context = history::outbox_context(&self.selection, &self.source);
                        history::saved_dependency(
                            store,
                            context,
                            &self.source,
                            scope,
                            dependency,
                            rng,
                        )
                    },
                    || {
                        if session.signal.is_cancelled() || frame.current_data().is_none() {
                            return Err(refused());
                        }
                        governed.require_current(store, &self.source)?;
                        Ok(())
                    },
                )
            })
            .unwrap_or(Some(false));
        self.restore_first_add(&session.signal)?;
        if !ingress.owns_frame(frame) {
            return Err(refused());
        }
        Ok(NativeTextFounderFrameOutcome::Retained(
            NativeTextFounderReceived::from_original(&original, archived),
        ))
    }

    pub(in crate::store::critical_namespaces::native_registry) fn recover_selected_dependency(
        &mut self,
        channel: u128,
        session: &FounderReceiveSession,
        response: &NativeTextFounderDependencyResponse,
        (carrier, admit_mutation): (
            Option<NativeChatMemberDependencyExchange>,
            &dyn crate::store::FounderArchiveAdmission,
        ),
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextFounderFrameOutcome, AppError> {
        let require_response = || response.require_current(session).map_err(|_| refused());
        require_response()?;
        self.restore_first_add(&session.signal)?;
        let scope = self.history_scope(channel)?;
        let (verifier, _) = self.governed.as_ref().ok_or_else(refused)?.member_parts()?;
        let sdk = self.sdk.as_ref().ok_or_else(refused)?;
        let (mount, _fee) = self.work.recovered_founder_resources(FRAME_FEE)?;
        let access = sdk.select_member_dependency_access(verifier, channel, self.local.0, mount)?;
        let Some(original) = response.open(access, session).map_err(|_| refused())? else {
            return Ok(NativeTextFounderFrameOutcome::Unresolved);
        };
        let ledger = self
            .received
            .as_mut()
            .filter(|ledger| {
                Arc::ptr_eq(&ledger.session.identity, &session.identity)
                    && ledger.ledger.realm().channel() == channel
            })
            .ok_or_else(refused)?;
        let selection = &self.selection;
        let source = &self.source;
        let work = &self.work;
        let status = work.with_recovered_founder_store(selection, source, |store| {
            project_founder_member(
                &original,
                verifier,
                &mut ledger.ledger,
                &ledger.view,
                (store, scope, admit_mutation),
                |id| {
                    let context = history::outbox_context(selection, source);
                    history::saved_dependency(store, context, source, scope, id, rng)
                },
                || {
                    require_response().map_err(|_| V2MemberSaveRefusal::Stale)?;
                    work.require_recovered_founder_source(selection, source)
                        .map_err(|_| V2MemberSaveRefusal::Stale)
                },
            )
            .map_err(|failure| match failure {
                V2MemberSaveRefusal::Busy => AppError::ChatV2ArchiveBusy,
                _ => refused(),
            })
        })?;
        if let (Some(carrier), Some(id)) = (carrier, first_founder_missing(&status)) {
            require_response()?;
            let (mount, fee) = work.recovered_founder_resources(FOUNDER_DEPENDENCY_REPLY_FEE)?;
            let access =
                sdk.select_member_dependency_access(verifier, channel, self.local.0, mount)?;
            let request = prepare_founder_dependency_exchange(
                access,
                carrier,
                (session, Some(response)),
                &original,
                id,
                fee,
                rng,
            )
            .map_err(|_| refused())?;
            self.restore_first_add(&session.signal)?;
            return Ok(NativeTextFounderFrameOutcome::NeedsDependency(request));
        }
        if status != V2OrdinaryReceiveStatus::Retained(ActionState::Valid) {
            return Ok(NativeTextFounderFrameOutcome::Unresolved);
        }
        self.restore_first_add(&session.signal)?;
        let ledger = self
            .received
            .as_ref()
            .filter(|ledger| Arc::ptr_eq(&ledger.session.identity, &session.identity))
            .ok_or_else(refused)?;
        let governed = self.governed.as_mut().ok_or_else(refused)?;
        let archived = self
            .work
            .with_recovered_founder_store(&self.selection, &self.source, |store| {
                archive_founder_member(
                    (store, scope),
                    &original,
                    &ledger.ledger,
                    rng,
                    |dependency, rng| {
                        let context = history::outbox_context(&self.selection, &self.source);
                        history::saved_dependency(
                            store,
                            context,
                            &self.source,
                            scope,
                            dependency,
                            rng,
                        )
                    },
                    || {
                        require_response()?;
                        governed.require_current(store, &self.source)?;
                        Ok(())
                    },
                )
            })
            .unwrap_or(Some(false));
        self.restore_first_add(&session.signal)?;
        require_response()?;
        Ok(NativeTextFounderFrameOutcome::Retained(
            NativeTextFounderReceived::from_original(&original, archived),
        ))
    }

    pub fn read_member_projection(
        &mut self,
        channel: u128,
        create: ActionId,
        caller: &RequestCancellation,
    ) -> Result<Option<NativeTextJoinedMemberProjection>, AppError> {
        self.restore_first_add(caller)?;
        let Some(received) = self.received.as_ref().filter(|received| {
            !received.session.signal.is_cancelled()
                && received.ledger.realm().channel() == channel
                && received.ledger.action(create).is_some()
        }) else {
            return Ok(None);
        };
        let (_, fee) = self
            .work
            .recovered_founder_resources(2 * MAX_ACTION_BYTES + 32 * 1024)?;
        let projection = received
            .ledger
            .guarded_projection(create, FOUNDER_RECEIVE_WORK)
            .map_err(|_| refused())?;
        let signal = caller.clone().with_authority(Arc::new(ProjectionScope {
            receiver: received.session.signal.clone(),
            caller: caller.clone(),
        }));
        if signal.is_cancelled() {
            return Err(refused());
        }
        Ok(Some(NativeTextJoinedMemberProjection::from_live(
            projection, signal, fee,
        )))
    }
}
