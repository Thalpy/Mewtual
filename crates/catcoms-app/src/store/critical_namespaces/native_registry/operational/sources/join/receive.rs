//! Bind ciphertext from the original native actor to the actual registered row.
use super::*;
use catcoms_net::{
    NativeChatMemberFrame, NativeChatMemberFrameObservation, NativeChatMemberGossip,
    NativeChatMemberGossipError, NativeChatMemberGossipEvent, NativeChatMemberIngress,
};
use catcoms_replication::chat::{
    ActionId, ChatLedger, GuardedMessageProjection, GuardedProjection, LedgerLimits,
    ProjectionGroupView, ProjectionView, Realm, V2GeneralPrefixVerifier, V2OrdinaryReceiveStatus,
    MAX_PARENTS,
};
use catcoms_rt::Topic;
use catcoms_sync::owner_v2::{SelectedV2MemberEnvelope, MAX_V2_MEMBER_TRANSPORT_BYTES};
use catcoms_sync::PreparedV2JoinedPrefix;

const INGRESS_FEE: usize = 8 * 1024;
// Padded plaintext, canonical signed action and its original frame may overlap.
const OPEN_FEE: usize = 4 * MAX_V2_MEMBER_TRANSPORT_BYTES + 4096;
const MAX_RECEIVED_ACTIONS: usize = 64;
const MAX_RECEIVED_BYTES: usize = 1024 * 1024;
const RECEIVE_WORK: usize = 4 * MAX_RECEIVED_ACTIONS + MAX_PARENTS;
// Allow two copies of canonical originals and separately budget the ledger
// entries, dependency edges, authority sets, tokens, and bounded work indices.
const LEDGER_FEE: usize = 2 * MAX_RECEIVED_BYTES
    + MAX_RECEIVED_ACTIONS * (MAX_PARENTS + 2) * 256
    + V2GeneralPrefixVerifier::ordinary_authorization_temporary_layout_bytes()
    + 16 * 1024;

/// One subscription can attach after the metadata slot and SDK loan leave.
pub struct NativeTextJoinedMemberIngressPreparation {
    ingress: NativeChatMemberIngress,
    topic: Topic,
    signal: RequestCancellation,
    invocation: watch::Sender<bool>,
}

pub struct NativeTextJoinedMemberReceiver {
    _invocation: watch::Sender<bool>,
    receiver: NativeChatMemberGossip,
}

impl NativeTextJoinedMemberIngressPreparation {
    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn topic_for_test(&self) -> Topic {
        self.topic.clone()
    }

    pub async fn open(self) -> Result<NativeTextJoinedMemberReceiver, NativeChatMemberGossipError> {
        let receiver = self
            .ingress
            .open(self.topic, MAX_V2_MEMBER_TRANSPORT_BYTES, self.signal)
            .await?;
        Ok(NativeTextJoinedMemberReceiver {
            _invocation: self.invocation,
            receiver,
        })
    }
}

impl NativeTextJoinedMemberReceiver {
    pub async fn next(&mut self) -> Option<NativeChatMemberGossipEvent> {
        self.receiver.next().await
    }
}

/// An opened original and its paid buffer. The receiver ledger still checks
/// the actual Core Admission and dependencies before revealing the action.
pub struct NativeTextJoinedReceivedOriginal {
    opened: SelectedV2MemberEnvelope,
    selection: NativeTextJoinedRowSelection,
    frame: NativeChatMemberFrameObservation,
    _fee: CatalogReadReservation,
}

impl NativeTextJoinedReceivedOriginal {
    pub fn is_for_original_row_cached(&self, selection: &NativeTextJoinedRowSelection) -> bool {
        self.selection.same_original(selection)
            && selection.is_current_cached()
            && self.frame.is_current_cached()
            && self.opened.is_current_cached()
    }

    /// Exact routing ID to use for dependency recovery, before projection.
    pub fn action_id_data(&self) -> catcoms_replication::chat::ActionId {
        self.opened.action_data().id()
    }
}

/// Paid current ledger for one channel in the original selected joined row.
/// The durable history owner can replay verified originals into a fresh view.
pub struct NativeTextJoinedMemberLedger {
    _invocation: watch::Sender<bool>,
    view: ProjectionView,
    ledger: ChatLedger,
    selection: NativeTextJoinedRowSelection,
    signal: RequestCancellation,
    _fee: CatalogReadReservation,
}

impl NativeTextJoinedMemberLedger {
    /// Prepare the exact view and bounded body outside the final UI guard.
    /// One paid query can borrow this ledger at a time.
    pub fn prepare_message_projection(
        &mut self,
        create: ActionId,
    ) -> Result<NativeTextJoinedMemberProjection<'_>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if self.signal.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        let projection = self
            .ledger
            .guarded_projection(create, RECEIVE_WORK)
            .map_err(|_| Refusal::Failed)?;
        Ok(NativeTextJoinedMemberProjection {
            projection,
            signal: &self.signal,
        })
    }
}

pub struct NativeTextJoinedMemberProjection<'a> {
    projection: GuardedMessageProjection,
    signal: &'a RequestCancellation,
}

impl NativeTextJoinedMemberProjection<'_> {
    /// Resolve inside the already acquired outer native UI guard.
    pub fn with_publication<T>(
        &self,
        publish: impl FnOnce(&GuardedProjection) -> T,
    ) -> Result<T, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        self.projection
            .with_publication(|result| {
                if self.signal.is_cancelled() {
                    return Err(Refusal::Cancelled);
                }
                Ok(publish(result))
            })
            .map_err(|_| Refusal::Stale)?
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    pub fn prepare_member_ingress(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        caller: RequestCancellation,
    ) -> Result<NativeTextJoinedMemberIngressPreparation, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let ingress = self.require_member_ingress()?;
        let (topic, observed, authentication, fee) =
            self.with_member_receiver(selection, &caller, |metadata, node, prefix, _| {
                let fee = metadata
                    .reserve_join_node(INGRESS_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let topic = node
                    .selected_member_topic_data(prefix, channel)
                    .map_err(|_| Refusal::Unavailable)?;
                let observed = node
                    .observe_member_publication(prefix)
                    .map_err(|_| Refusal::Unavailable)?;
                let authentication = metadata
                    .authentication_negative()
                    .map_err(|_| Refusal::Unavailable)?;
                Ok((topic, observed, authentication, fee))
            })?;
        let (invocation, live) = watch::channel(false);
        let authority = Arc::new(JoinEffect {
            scope: self.original.scope,
            ui: self.original.command.ui.into_effect_negative(),
            authentication,
            invocation: live,
            caller: caller.clone(),
            row: Some(selection.clone()),
        });
        let signal = caller
            .with_authority(authority)
            .retain_custody(Arc::new(JoinFramePayment { _fee: fee }));
        let signal = observed.constrain(signal);
        Ok(NativeTextJoinedMemberIngressPreparation {
            ingress,
            topic,
            signal,
            invocation,
        })
    }

    pub fn open_member_frame(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        frame: &NativeChatMemberFrame,
        caller: &RequestCancellation,
    ) -> Result<NativeTextJoinedReceivedOriginal, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let ingress = self.require_member_ingress()?;
        if !ingress.owns_frame(frame) {
            return Err(Refusal::Stale);
        }
        self.with_member_receiver(selection, caller, |metadata, node, prefix, _| {
            let (topic, _, bytes) = frame.current_data().ok_or(Refusal::Stale)?;
            let fee = metadata
                .reserve_join_node(OPEN_FEE)
                .map_err(|_| Refusal::Busy)?;
            let opened = node
                .open_selected_member_envelope(prefix, channel, topic, bytes)
                .map_err(|_| Refusal::Failed)?;
            if frame.current_data().is_none() {
                return Err(Refusal::Stale);
            }
            Ok(NativeTextJoinedReceivedOriginal {
                opened,
                selection: selection.clone(),
                frame: frame.observe(),
                _fee: fee,
            })
        })
    }

    pub fn prepare_member_ledger(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        caller: RequestCancellation,
    ) -> Result<NativeTextJoinedMemberLedger, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let (view, ledger, observed, authentication, fee) =
            self.with_member_receiver(selection, &caller, |metadata, node, prefix, group| {
                let fee = metadata
                    .reserve_join_node(LEDGER_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let verifier = prefix
                    .selected_member_verifier()
                    .map_err(|_| Refusal::Unavailable)?;
                let group_data = &verifier.replay_data().profile().spec().group;
                let realm = Realm::new(group_data, channel).map_err(|_| Refusal::Failed)?;
                let mut ledger = ChatLedger::new(
                    realm,
                    LedgerLimits {
                        max_actions: MAX_RECEIVED_ACTIONS,
                        max_action_bytes: MAX_RECEIVED_BYTES,
                    },
                )
                .map_err(|_| Refusal::Failed)?;
                let mut view = ledger
                    .begin_projection_view_in(group)
                    .map_err(|_| Refusal::Unavailable)?;
                if !view.try_ready() {
                    return Err(Refusal::Busy);
                }
                let observed = node
                    .observe_member_publication(prefix)
                    .map_err(|_| Refusal::Unavailable)?;
                let authentication = metadata
                    .authentication_negative()
                    .map_err(|_| Refusal::Unavailable)?;
                Ok((view, ledger, observed, authentication, fee))
            })?;
        let (invocation, live) = watch::channel(false);
        let authority = Arc::new(JoinEffect {
            scope: self.original.scope,
            ui: self.original.command.ui.into_effect_negative(),
            authentication,
            invocation: live,
            caller: caller.clone(),
            row: Some(selection.clone()),
        });
        let signal = observed.constrain(caller.with_authority(authority));
        Ok(NativeTextJoinedMemberLedger {
            _invocation: invocation,
            view,
            ledger,
            selection: selection.clone(),
            signal,
            _fee: fee,
        })
    }

    /// Admit a live network original into this exact channel ledger. A missing
    /// dependency leaves the child outside the ledger for bounded recovery.
    pub fn project_received_member(
        self,
        ledger: &mut NativeTextJoinedMemberLedger,
        original: &NativeTextJoinedReceivedOriginal,
        caller: &RequestCancellation,
    ) -> Result<V2OrdinaryReceiveStatus, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if ledger.signal.is_cancelled() || !original.is_for_original_row_cached(&ledger.selection) {
            return Err(Refusal::Stale);
        }
        self.with_member_receiver(&ledger.selection, caller, |_, _, prefix, _| {
            let verifier = prefix
                .selected_member_verifier()
                .map_err(|_| Refusal::Unavailable)?;
            if ledger.signal.is_cancelled()
                || !original.is_for_original_row_cached(&ledger.selection)
            {
                return Err(Refusal::Stale);
            }
            original
                .opened
                .project_from_ledger(verifier, &mut ledger.ledger, &ledger.view, RECEIVE_WORK)
                .map_err(|_| Refusal::Failed)
        })
    }

    fn require_member_ingress(&self) -> Result<NativeChatMemberIngress, V2MemberSaveRefusal> {
        self.original
            .tcp
            .upgrade()
            .ok_or(V2MemberSaveRefusal::Unavailable)?
            .capture_member_ingress()
            .map_err(|_| V2MemberSaveRefusal::Unavailable)
    }

    fn with_member_receiver<T>(
        &self,
        selection: &NativeTextJoinedRowSelection,
        caller: &RequestCancellation,
        read: impl FnOnce(
            &MetadataWork,
            &ClosedV2JoinedNode<catcoms_net::MeshService, R>,
            &PreparedV2JoinedPrefix,
            &ProjectionGroupView,
        ) -> Result<T, V2MemberSaveRefusal>,
    ) -> Result<T, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        self.original
            .require_current(caller)
            .map_err(|_| Refusal::Cancelled)?;
        if !selection.is_current_cached() {
            return Err(Refusal::Stale);
        }
        let metadata = self.original.work.as_ref().ok_or(Refusal::Unavailable)?;
        let pending = self
            .original
            .pending
            .upgrade()
            .ok_or(Refusal::Unavailable)?;
        let mounted = self.mounted.upgrade().ok_or(Refusal::Unavailable)?;
        let result = {
            let pending = pending.state.try_lock().map_err(|_| Refusal::Busy)?;
            let slot = mounted.try_lock().map_err(|_| Refusal::Busy)?;
            let runtime = slot
                .as_ref()
                .filter(|runtime| runtime.matches_selection(selection))
                .ok_or(Refusal::Stale)?;
            let data = runtime.registration_data().ok_or(Refusal::Unavailable)?;
            let node = runtime.node.as_ref().ok_or(Refusal::Unavailable)?;
            metadata.with_registered_joined_member(
                data,
                runtime.registration.as_deref(),
                selection.original.row,
                |_, store| {
                    if node
                        .selected_join_data()
                        .map_err(|_| Refusal::Unavailable)?
                        != data.selected_data()
                    {
                        return Err(Refusal::Stale);
                    }
                    let recovered = runtime
                        .recovered
                        .map(|data| store.capture_recovered_member_prefix(data))
                        .transpose()
                        .map_err(|_| Refusal::Unavailable)?;
                    let (prefix, view) = match &recovered {
                        Some(prefix) => prefix.member_parts(),
                        None => pending
                            .io
                            .as_ref()
                            .ok_or(Refusal::Unavailable)?
                            .borrow_warm_member_prefix(store)
                            .map_err(|_| Refusal::Unavailable)?,
                    };
                    read(metadata, node, prefix, view)
                },
            )?
        };
        self.original
            .require_current(caller)
            .map_err(|_| Refusal::Cancelled)?;
        if !selection.is_current_cached() {
            return Err(Refusal::Stale);
        }
        Ok(result)
    }
}
