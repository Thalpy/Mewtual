//! Live ordinary text from the recovered founder's exact reserved mesh.
use super::*;
use crate::store::persistence::native_registry::network::NetworkDependency;
use crate::store::persistence::v2_member_transport::{
    archive_founder_member, project_founder_member, FOUNDER_RECEIVE_ACTIONS, FOUNDER_RECEIVE_BYTES,
    FOUNDER_RECEIVE_LEDGER_FEE, FOUNDER_RECEIVE_WORK,
};
use crate::store::{
    NativeTextFounderReceived, NativeTextJoinedMemberProjection, V2MemberSaveRefusal,
};
use catcoms_net::{
    NativeChatMemberFrame, NativeChatMemberGossip, NativeChatMemberGossipError,
    NativeChatMemberGossipEvent, NativeChatMemberIngress,
};
use catcoms_replication::chat::{
    ActionId, ChatLedger, LedgerLimits, ProjectionView, Realm, MAX_ACTION_BYTES,
};
use catcoms_rt::Topic;
use catcoms_sync::owner_v2::MAX_V2_MEMBER_TRANSPORT_BYTES;

const INGRESS_FEE: usize = 2 * MAX_V2_MEMBER_TRANSPORT_BYTES + 8192;
const FRAME_FEE: usize = 5 * MAX_V2_MEMBER_TRANSPORT_BYTES + 4096;

pub struct NativeTextColdFounderMemberIngress {
    ingress: NativeChatMemberIngress,
    original: Arc<NetworkDependency>,
    topic: Topic,
    channel: u128,
    signal: RequestCancellation,
    owner: watch::Sender<()>,
    scope: Arc<IngressScope>,
}

pub struct NativeTextColdFounderMemberReceiver {
    _owner: watch::Sender<()>,
    receiver: NativeChatMemberGossip,
    ingress: NativeChatMemberIngress,
    original: Arc<NetworkDependency>,
    channel: u128,
    signal: RequestCancellation,
    scope: Arc<IngressScope>,
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
                self.signal.clone(),
            )
            .await?;
        Ok(NativeTextColdFounderMemberReceiver {
            _owner: self.owner,
            receiver,
            ingress: self.ingress,
            original: self.original,
            channel: self.channel,
            signal: self.signal,
            scope: self.scope,
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
    scope: Arc<IngressScope>,
    _fee: CatalogReadReservation,
}

impl NativeTextColdFounderSource {
    pub fn prepare_member_ingress(
        &mut self,
        channel: u128,
        caller: RequestCancellation,
    ) -> Result<NativeTextColdFounderMemberIngress, AppError> {
        self.restore_first_add(&caller)?;
        let mesh = self.actor.as_ref().ok_or_else(refused)?.borrow()?;
        let ingress = mesh.native_chat_member_ingress().ok_or_else(refused)?;
        let (mount, fee) = self.storage.index.joined_exchange_resources(INGRESS_FEE)?;
        let (verifier, _) = self.governed.as_ref().ok_or_else(refused)?.member_parts()?;
        let sdk = self.sdk.as_ref().ok_or_else(refused)?;
        let (topic, observation) = sdk.prepare_member_topic(verifier, channel, mount.clone())?;
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
            signal,
            owner,
            scope,
        })
    }

    pub fn receive_member_frame(
        &mut self,
        receiver: &NativeTextColdFounderMemberReceiver,
        frame: &NativeChatMemberFrame,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<NativeTextFounderReceived>, AppError> {
        if !Arc::ptr_eq(&receiver.original, &self.selection.network)
            || !receiver.ingress.owns_frame(frame)
        {
            return Err(refused());
        }
        self.restore_first_add(&receiver.signal)?;
        let scope = self.history_scope(receiver.channel)?;
        let (verifier, group) = self.governed.as_ref().ok_or_else(refused)?.member_parts()?;
        let sdk = self.sdk.as_ref().ok_or_else(refused)?;
        let (mount, _payment) = self.storage.index.joined_exchange_resources(FRAME_FEE)?;
        let (topic, _, wire) = frame.current_data().ok_or_else(refused)?;
        let original = sdk.open_member_envelope(verifier, receiver.channel, topic, wire, mount)?;
        if self
            .received
            .as_ref()
            .is_some_and(|ledger| !ledger.scope.authorized())
        {
            self.received = None;
        }
        if self.received.is_none() {
            let (_, fee) = self
                .storage
                .index
                .joined_exchange_resources(FOUNDER_RECEIVE_LEDGER_FEE)?;
            let realm = Realm::new(&scope.group, receiver.channel).map_err(|_| refused())?;
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
                scope: receiver.scope.clone(),
                _fee: fee,
            });
        }
        let selection = &self.selection;
        let source = &self.source;
        let index = &self.storage.index;
        let ledger = self
            .received
            .as_mut()
            .filter(|ledger| Arc::ptr_eq(&ledger.scope, &receiver.scope))
            .ok_or_else(refused)?;
        let check_lifetime = || {
            if receiver.signal.is_cancelled() || frame.current_data().is_none() {
                return Err(refused());
            }
            index.require_recovered_founder_source(selection, source)
        };
        let retained = index.with_recovered_founder_store(selection, source, |store| {
            project_founder_member(
                &original,
                verifier,
                &mut ledger.ledger,
                &ledger.view,
                |id| {
                    let context = history::outbox_context(selection, source);
                    history::saved_dependency(store, context, source, scope, id, rng)
                },
                || check_lifetime().map_err(|_| V2MemberSaveRefusal::Stale),
            )
            .map_err(|_| refused())
        })?;
        if !retained {
            return Ok(None);
        }
        self.restore_first_add(&receiver.signal)?;
        let governed = self.governed.as_mut().ok_or_else(refused)?;
        let archived = self
            .storage
            .index
            .with_recovered_founder_store(&self.selection, &self.source, |store| {
                archive_founder_member(store, scope, &original, rng, || {
                    if receiver.signal.is_cancelled() || frame.current_data().is_none() {
                        return Err(refused());
                    }
                    governed.require_current(store, &self.source)?;
                    Ok(())
                })
            })
            .unwrap_or(Some(false));
        self.restore_first_add(&receiver.signal)?;
        if !receiver.ingress.owns_frame(frame) {
            return Err(refused());
        }
        let action = original.action_data();
        Ok(Some(NativeTextFounderReceived {
            action: action.id(),
            projection_target: action.spec().body.create().unwrap_or_else(|| action.id()),
            archived,
        }))
    }

    pub fn read_member_projection(
        &mut self,
        channel: u128,
        create: ActionId,
        caller: &RequestCancellation,
    ) -> Result<Option<NativeTextJoinedMemberProjection>, AppError> {
        self.restore_first_add(caller)?;
        let Some(received) = self.received.as_ref().filter(|received| {
            received.scope.authorized()
                && received.ledger.realm().channel() == channel
                && received.ledger.action(create).is_some()
        }) else {
            return Ok(None);
        };
        let (_, fee) = self
            .storage
            .index
            .joined_exchange_resources(2 * MAX_ACTION_BYTES + 32 * 1024)?;
        let projection = received
            .ledger
            .guarded_projection(create, FOUNDER_RECEIVE_WORK)
            .map_err(|_| refused())?;
        let signal = caller.clone().with_authority(Arc::new(IngressScope {
            lifetime: received.scope.lifetime.clone(),
            mount: received.scope.caller.clone(),
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
