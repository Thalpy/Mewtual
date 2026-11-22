//! Bind ciphertext from the original native actor to the actual registered row.
use super::*;
use catcoms_net::{
    NativeChatMemberFrame, NativeChatMemberFrameObservation, NativeChatMemberGossip,
    NativeChatMemberGossipError, NativeChatMemberGossipEvent, NativeChatMemberIngress,
};
use catcoms_rt::Topic;
use catcoms_sync::owner_v2::{SelectedV2MemberEnvelope, MAX_V2_MEMBER_TRANSPORT_BYTES};
use catcoms_sync::PreparedV2JoinedPrefix;

const INGRESS_FEE: usize = 8 * 1024;
// Padded plaintext, canonical signed action and its original frame may overlap.
const OPEN_FEE: usize = 4 * MAX_V2_MEMBER_TRANSPORT_BYTES + 4096;

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
            self.with_member_receiver(selection, &caller, |metadata, node, prefix| {
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
        self.with_member_receiver(selection, caller, |metadata, node, prefix| {
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
                    let (prefix, _) = match &recovered {
                        Some(prefix) => prefix.member_parts(),
                        None => pending
                            .io
                            .as_ref()
                            .ok_or(Refusal::Unavailable)?
                            .borrow_warm_member_prefix(store)
                            .map_err(|_| Refusal::Unavailable)?,
                    };
                    read(metadata, node, prefix)
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
