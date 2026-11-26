//! Bind ciphertext from the original native actor to the actual registered row.
catcoms_rt::redacted_debug! {
    NativeTextJoinedMemberIngressPreparation;
    NativeTextJoinedMemberReceiver;
    NativeTextJoinedReceivedOriginal;
    NativeTextJoinedMemberLedger;
    NativeTextJoinedMemberProjection;
    NativeTextJoinedArchivedActions;
}

use super::*;
use crate::store::persistence::v2_member_outbox::{member_outbox_key, MemberSavedOriginal};
use crate::store::persistence::v2_member_transport::replay_local_member_dependencies;
use crate::store::persistence::v2_shared_history::{
    PreparedSharedHistoryRecord, SharedHistoryActionPage, SharedHistoryActionQuery,
    SharedHistoryMessageSelection, SharedHistoryProjectionLease, SharedHistoryReplay,
    SharedHistoryScope, SHARED_HISTORY_RECORD_FEE,
};
use crate::store::ServerStore;
use catcoms_net::{
    NativeChatMemberFrame, NativeChatMemberFrameObservation, NativeChatMemberGossip,
    NativeChatMemberGossipError, NativeChatMemberGossipEvent, NativeChatMemberIngress,
};
use catcoms_replication::chat::{
    ActionId, ChatLedger, GuardedMessageProjection, GuardedProjection, LedgerLimits,
    ProjectionGroupView, ProjectionView, Realm, V2GeneralPrefixVerifier, V2OrdinaryReceiveStatus,
    MAX_ACTION_BYTES, MAX_PARENTS,
};
use catcoms_rt::Topic;
use catcoms_sync::owner_v2::{SelectedV2MemberEnvelope, MAX_V2_MEMBER_TRANSPORT_BYTES};
use catcoms_sync::PreparedV2JoinedPrefix;

mod contact;
mod dependencies;
mod governing;
pub use dependencies::{
    NativeTextJoinedDependencyAttachRefusal, NativeTextJoinedDependencyExchange,
    NativeTextJoinedDependencyProvider, NativeTextJoinedDependencyResponse,
};
pub use governing::{
    NativeTextJoinedGoverningExchange, NativeTextJoinedGoverningIncoming,
    NativeTextJoinedGoverningProvider, NativeTextJoinedGoverningResponse,
};

const INGRESS_FEE: usize = 8 * 1024;
// Padded plaintext, canonical signed action and its original frame may overlap.
const OPEN_FEE: usize = 4 * MAX_V2_MEMBER_TRANSPORT_BYTES + 4096;
const MAX_RECEIVED_ACTIONS: usize = 64;
const MAX_RECEIVED_BYTES: usize = 1024 * 1024;
const RECEIVE_WORK: usize = 4 * MAX_RECEIVED_ACTIONS + MAX_PARENTS;
const PROJECTION_FEE: usize = 2 * MAX_ACTION_BYTES + 32 * 1024;
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
    origin: ReceiveOrigin,
    _fee: CatalogReadReservation,
}

enum ReceiveOrigin {
    Gossip(NativeChatMemberFrameObservation),
    Dependency {
        route: catcoms_net::NativeChatMemberDependencyObservation,
        frame_depth: Option<u8>,
        signal: RequestCancellation,
        _invocation: watch::Sender<bool>,
    },
}

impl ReceiveOrigin {
    fn is_current_cached(&self) -> bool {
        match self {
            Self::Gossip(frame) => frame.is_current_cached(),
            Self::Dependency { route, signal, .. } => {
                route.is_current_cached() && !signal.is_cancelled()
            }
        }
    }
}

impl NativeTextJoinedReceivedOriginal {
    pub fn is_for_original_row_cached(&self, selection: &NativeTextJoinedRowSelection) -> bool {
        self.selection.same_original(selection)
            && selection.is_current_cached()
            && self.origin.is_current_cached()
            && self.opened.is_current_cached()
    }

    /// Exact routing ID to use for dependency recovery, before projection.
    pub fn action_id_data(&self) -> catcoms_replication::chat::ActionId {
        self.opened.action_data().id()
    }

    pub fn projection_target_data(&self) -> ActionId {
        let action = self.opened.action_data();
        action.spec().body.create().unwrap_or_else(|| action.id())
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

/// Same native files, selected Core view and installed SDK in one synchronous
/// physical read. No capability borrows these parts across an await.
struct ReceiverSource<'a, R: CryptoRngCore> {
    metadata: &'a MetadataWork,
    node: &'a ClosedV2JoinedNode<catcoms_net::MeshService, R>,
    prefix: &'a PreparedV2JoinedPrefix,
    view: &'a ProjectionGroupView,
    store: &'a ServerStore,
    warm: Option<&'a JoinedPendingIo>,
    registration: NativeTextJoinedRegistrationData,
    founder: Option<PeerId>,
}

pub(in crate::store::critical_namespaces::native_registry) struct NativeTextJoinedSharedHistoryRecord
{
    pub(in crate::store::critical_namespaces::native_registry) action: ActionId,
    pub(in crate::store::critical_namespaces::native_registry) manifest:
        catcoms_storage::private_chat_page::PrivateChatPageId,
    pub(in crate::store::critical_namespaces::native_registry) bundle: Zeroizing<Vec<u8>>,
    _fee: CatalogReadReservation,
}

impl NativeTextJoinedMemberLedger {
    pub fn is_current_cached(&self) -> bool {
        !self.signal.is_cancelled() && self.selection.is_current_cached()
    }

    pub fn read_message_projection(
        &self,
        create: ActionId,
    ) -> Result<Option<NativeTextJoinedMemberProjection>, V2MemberSaveRefusal> {
        if !self.is_current_cached() {
            return Err(V2MemberSaveRefusal::Stale);
        }
        if self.ledger.action(create).is_none() {
            return Ok(None);
        }
        self.prepare_message_projection(create).map(Some)
    }

    /// Prepare the exact view and bounded body outside the final UI guard.
    /// One paid query can borrow this ledger at a time.
    pub fn prepare_message_projection(
        &self,
        create: ActionId,
    ) -> Result<NativeTextJoinedMemberProjection, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if self.signal.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        let fee = self
            ._fee
            .reserve_more(PROJECTION_FEE)
            .map_err(|_| Refusal::Busy)?;
        let projection = self
            .ledger
            .guarded_projection(create, RECEIVE_WORK)
            .map_err(|_| Refusal::Failed)?;
        Ok(NativeTextJoinedMemberProjection {
            projection,
            signal: self.signal.clone(),
            archive: None,
            _fee: fee,
        })
    }
}

pub struct NativeTextJoinedMemberProjection {
    projection: GuardedMessageProjection,
    signal: RequestCancellation,
    archive: Option<Box<SharedHistoryProjectionLease>>,
    _fee: CatalogReadReservation,
}

pub struct NativeTextJoinedArchivedActions(pub(in crate::store) SharedHistoryActionPage);

impl NativeTextJoinedArchivedActions {
    pub fn actions(&self) -> &[ActionId] {
        &self.0.actions
    }

    pub fn has_more(&self) -> bool {
        self.0.more
    }
}

impl NativeTextJoinedMemberProjection {
    /// Exact member of the replayed target's selected archive action set.
    /// Historical dependencies and local outbox entries cannot satisfy it.
    pub fn archived_includes(&self, action: ActionId) -> bool {
        self.archive
            .as_ref()
            .is_some_and(|archive| archive.selected.includes(action))
    }

    pub(in crate::store::critical_namespaces::native_registry) fn from_live(
        projection: GuardedMessageProjection,
        signal: RequestCancellation,
        fee: CatalogReadReservation,
    ) -> Self {
        Self {
            projection,
            signal,
            archive: None,
            _fee: fee,
        }
    }

    pub(in crate::store::critical_namespaces::native_registry) fn from_archive(
        projection: GuardedMessageProjection,
        signal: RequestCancellation,
        fee: CatalogReadReservation,
        archive: Box<SharedHistoryProjectionLease>,
    ) -> Self {
        Self {
            projection,
            signal,
            archive: Some(archive),
            _fee: fee,
        }
    }

    /// Resolve inside the already acquired outer native UI guard.
    pub fn with_publication<T>(
        &self,
        publish: impl FnOnce(&GuardedProjection) -> T,
    ) -> Result<T, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let guarded = || {
            self.projection
                .with_publication(|result| {
                    if self.signal.is_cancelled() {
                        return Err(Refusal::Cancelled);
                    }
                    Ok(publish(result))
                })
                .map_err(|_| Refusal::Stale)?
        };
        match &self.archive {
            Some(archive) => archive
                .with_publication(guarded)
                .map_err(|_| Refusal::Stale)?,
            None => guarded(),
        }
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    pub fn governed_roster(
        self,
        selection: &NativeTextJoinedRowSelection,
        caller: &RequestCancellation,
    ) -> Result<crate::store::NativeTextRoster, V2MemberSaveRefusal> {
        self.with_member_receiver(selection, caller, |source| {
            let verifier = source
                .prefix
                .selected_member_verifier()
                .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
            let replay = verifier.replay_data();
            let own = source.prefix.own_admission_data();
            if source
                .node
                .selected_join_data()
                .map_err(|_| V2MemberSaveRefusal::Unavailable)?
                != (replay.header().id(), own, replay.mls_epoch_data())
            {
                return Err(V2MemberSaveRefusal::Stale);
            }
            let own =
                catcoms_replication::chat::owner_header::OwnerBodyId::from_bytes(*own.as_bytes());
            crate::store::NativeTextRoster::from_selected(source.store, replay, own)
                .map_err(|_| V2MemberSaveRefusal::Failed)
        })
    }

    pub fn archived_action_page(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        after: Option<ActionId>,
        limit: usize,
        caller: &RequestCancellation,
    ) -> Result<Option<NativeTextJoinedArchivedActions>, V2MemberSaveRefusal> {
        self.with_member_receiver(selection, caller, |source| {
            source
                .archived_actions(
                    channel,
                    SharedHistoryActionQuery {
                        target: None,
                        after,
                        limit,
                    },
                )
                .map(|page| page.map(NativeTextJoinedArchivedActions))
        })
    }

    /// Rebuild a bounded projection for one exact archived Create under the
    /// current joined SDK and governing view. The view outlives the response
    /// so the renderer must still enter the normal publication guard.
    pub fn archived_message_projection(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        create: ActionId,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<NativeTextJoinedMemberProjection>, V2MemberSaveRefusal> {
        let (ledger, selected) = self.prepare_member_ledger_with(
            selection,
            channel,
            caller.clone(),
            |source, ledger, view| {
                source.replay_archived_message(ledger, view, create, &caller, rng)
            },
        )?;
        let Some(selected) = selected else {
            return Ok(None);
        };
        let mut projection = ledger.prepare_message_projection(create)?;
        projection.archive = Some(Box::new(SharedHistoryProjectionLease {
            selected,
            _view: ledger.view,
            _ledger: ledger.ledger,
            _invocation: Some(ledger._invocation),
            _fee: ledger._fee,
        }));
        Ok(Some(projection))
    }

    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn mesh_peer_name_for_test(
        self,
    ) -> Option<String> {
        self.original.tcp.upgrade()?.peer_name_for_test()
    }

    pub fn prepare_member_ingress(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        caller: RequestCancellation,
    ) -> Result<NativeTextJoinedMemberIngressPreparation, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let ingress = self.require_member_ingress()?;
        let (topic, observed, authentication, fee) =
            self.with_member_receiver(selection, &caller, |source| {
                let ReceiverSource {
                    metadata,
                    node,
                    prefix,
                    ..
                } = source;
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
        self.open_member_frame_for(selection, channel, frame, caller)
    }

    fn open_member_frame_for(
        &self,
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
        self.with_member_receiver(selection, caller, |source| {
            let ReceiverSource {
                metadata,
                node,
                prefix,
                ..
            } = source;
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
                origin: ReceiveOrigin::Gossip(frame.observe()),
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
        self.prepare_member_ledger_with(selection, channel, caller, |_, _, _| Ok(()))
            .map(|(ledger, ())| ledger)
    }

    fn prepare_member_ledger_with<T>(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        caller: RequestCancellation,
        populate: impl FnOnce(
            &ReceiverSource<'_, R>,
            &mut ChatLedger,
            &ProjectionView,
        ) -> Result<T, V2MemberSaveRefusal>,
    ) -> Result<(NativeTextJoinedMemberLedger, T), V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let (view, ledger, observed, authentication, result, fee) =
            self.with_member_receiver(selection, &caller, |source| {
                let ReceiverSource {
                    metadata,
                    node,
                    prefix,
                    view: group,
                    ..
                } = source;
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
                let result = populate(&source, &mut ledger, &view)?;
                Ok((view, ledger, observed, authentication, result, fee))
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
        Ok((
            NativeTextJoinedMemberLedger {
                _invocation: invocation,
                view,
                ledger,
                selection: selection.clone(),
                signal,
                _fee: fee,
            },
            result,
        ))
    }

    /// Admit a live network original into this exact channel ledger. A missing
    /// dependency leaves the child outside the ledger for bounded recovery.
    pub fn project_received_member(
        &self,
        ledger: &mut NativeTextJoinedMemberLedger,
        original: &NativeTextJoinedReceivedOriginal,
        caller: &RequestCancellation,
    ) -> Result<V2OrdinaryReceiveStatus, V2MemberSaveRefusal> {
        self.project_received_for(ledger, original, caller)
    }

    /// Apply one exact frame on the same paid mounted row. Waiting children
    /// retain their original carrier for an addressed dependency attempt.
    pub fn receive_member_frame(
        &self,
        ledger: &mut NativeTextJoinedMemberLedger,
        frame: &NativeChatMemberFrame,
        caller: &RequestCancellation,
    ) -> Result<(NativeTextJoinedReceivedOriginal, V2OrdinaryReceiveStatus), V2MemberSaveRefusal>
    {
        let original = self.open_member_frame_for(
            &ledger.selection,
            ledger.ledger.realm().channel(),
            frame,
            caller,
        )?;
        let status = self.project_received_for(ledger, &original, caller)?;
        Ok((original, status))
    }

    fn project_received_for(
        &self,
        ledger: &mut NativeTextJoinedMemberLedger,
        original: &NativeTextJoinedReceivedOriginal,
        caller: &RequestCancellation,
    ) -> Result<V2OrdinaryReceiveStatus, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if ledger.signal.is_cancelled() || !original.is_for_original_row_cached(&ledger.selection) {
            return Err(Refusal::Stale);
        }
        self.with_member_receiver(&ledger.selection, caller, |source| {
            let verifier = source
                .prefix
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

    /// Restore a bounded causal slice from the original row's local receipts
    /// and selected archive. Replay supplies historical dependencies only.
    pub fn replay_saved_member_dependency(
        self,
        ledger: &mut NativeTextJoinedMemberLedger,
        action: ActionId,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<V2OrdinaryReceiveStatus>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if ledger.signal.is_cancelled() {
            return Err(Refusal::Stale);
        }
        self.with_member_receiver(&ledger.selection, caller, |source| {
            let verifier = source
                .prefix
                .selected_member_verifier()
                .map_err(|_| Refusal::Unavailable)?;
            let channel = ledger.ledger.realm().channel();
            let subscription = &ledger.signal;
            replay_local_member_dependencies(
                std::slice::from_ref(&action),
                (verifier, &mut ledger.ledger, &ledger.view),
                RECEIVE_WORK,
                (source.store, source.history_scope(channel)),
                |id| source.read_dependency(channel, id, rng),
                || {
                    if subscription.is_cancelled() || caller.is_cancelled() {
                        return Err(Refusal::Cancelled);
                    }
                    Ok(())
                },
            )
        })
    }

    /// Find one exact selected own original for dependency catch-up. A
    /// request handler still needs a distinct current sealed response and
    /// the original requesting peer's authenticated scope.
    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store::critical_namespaces::native_registry) fn read_member_dependency(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        action: ActionId,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<MemberSavedOriginal>, V2MemberSaveRefusal> {
        self.with_member_receiver(selection, caller, |source| {
            source.read_dependency(channel, action, rng)
        })
    }

    /// Freeze the exact admitted original into paid archive pages. Preparing
    /// these pages does not select them in the vault's shared history.
    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store::critical_namespaces::native_registry) fn prepare_shared_history_record(
        self,
        ledger: &NativeTextJoinedMemberLedger,
        original: &NativeTextJoinedReceivedOriginal,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedSharedHistoryRecord, V2MemberSaveRefusal> {
        require_archivable(ledger, original)?;
        self.with_member_receiver(&ledger.selection, caller, |source| {
            source.prepare_history_original(ledger, original, rng)
        })
    }

    /// Persist an exact already verified original if the row has the selected
    /// history layout. Only the disk selector supplies the saved receipt.
    pub fn archive_received_member(
        &self,
        ledger: &NativeTextJoinedMemberLedger,
        original: &NativeTextJoinedReceivedOriginal,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<catcoms_storage::private_chat_page::PrivateChatPageId>, V2MemberSaveRefusal>
    {
        use V2MemberSaveRefusal as Refusal;
        require_archivable(ledger, original)?;
        self.with_member_receiver(&ledger.selection, caller, |source| {
            if !source
                .store
                .has_mounted_shared_history()
                .map_err(|_| Refusal::Busy)?
            {
                return Ok(None);
            }
            let record = source.prepare_history_original(ledger, original, rng)?;
            let selected = source
                .store
                .save_shared_history_record(
                    source.history_scope(ledger.ledger.realm().channel()),
                    record.action,
                    record.manifest,
                    &record.bundle,
                    rng,
                )
                .map_err(|_| Refusal::Failed)?;
            require_archivable(ledger, original)?;
            Ok(Some(selected))
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

    /// Borrow the original selected local epoch to construct one request for
    /// its immediate successor. The caller retains its connected route.
    pub fn member_add_request_data(
        &self,
        selection: &NativeTextJoinedRowSelection,
        caller: &RequestCancellation,
    ) -> Result<catcoms_sync::V2MemberAddRequestData, V2MemberSaveRefusal> {
        self.with_member_receiver(selection, caller, |source| {
            source
                .node
                .member_add_request_data(source.prefix)
                .map_err(|_| V2MemberSaveRefusal::Unavailable)
        })
    }

    fn with_member_receiver<T>(
        &self,
        selection: &NativeTextJoinedRowSelection,
        caller: &RequestCancellation,
        read: impl FnOnce(ReceiverSource<'_, R>) -> Result<T, V2MemberSaveRefusal>,
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
                    node.selected_join_data()
                        .map_err(|_| Refusal::Unavailable)?;
                    if node.original_join_data() != data.selected_data() {
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
                    read(ReceiverSource {
                        metadata,
                        node,
                        prefix,
                        view,
                        store,
                        warm: pending.io.as_ref().filter(|_| runtime.recovered.is_none()),
                        registration: data,
                        founder: runtime.founder_contact.get().copied(),
                    })
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

impl<R: CryptoRngCore> ReceiverSource<'_, R> {
    fn archived_actions(
        &self,
        channel: u128,
        query: SharedHistoryActionQuery,
    ) -> Result<Option<SharedHistoryActionPage>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !self
            .store
            .has_mounted_shared_history()
            .map_err(|_| Refusal::Busy)?
        {
            return Ok(None);
        }
        self.store
            .shared_history_action_page(self.history_scope(channel), query)
            .map(Some)
            .map_err(|_| Refusal::Failed)
    }

    fn replay_archived_message(
        &self,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        create: ActionId,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<SharedHistoryMessageSelection>, V2MemberSaveRefusal> {
        let channel = ledger.realm().channel();
        SharedHistoryReplay {
            store: self.store,
            scope: self.history_scope(channel),
            verifier: self
                .prefix
                .selected_member_verifier()
                .map_err(|_| V2MemberSaveRefusal::Unavailable)?,
            caller,
        }
        .replay_message(ledger, view, create, |action| {
            self.read_dependency(channel, action, rng)
        })
    }

    fn history_scope(&self, channel: u128) -> SharedHistoryScope {
        let data = self.registration;
        SharedHistoryScope {
            vault: data.context.vault,
            group: data.group,
            profile: data.selected.profile,
            tenure: data.selected.initial_tenure,
            creation: data.selected.profile_creation,
            channel,
        }
    }

    fn prepare_history_original(
        &self,
        ledger: &NativeTextJoinedMemberLedger,
        original: &NativeTextJoinedReceivedOriginal,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedSharedHistoryRecord, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let fee = self
            .metadata
            .reserve_join_node(SHARED_HISTORY_RECORD_FEE)
            .map_err(|_| Refusal::Busy)?;
        let key = member_outbox_key(self.store).map_err(|_| Refusal::Unavailable)?;
        let record = PreparedSharedHistoryRecord::prepare(
            &self.history_scope(ledger.ledger.realm().channel()),
            &key,
            original.opened.admission_data(),
            original.opened.action_data(),
            original.opened.original_data(),
            rng,
        )
        .map_err(|_| Refusal::Failed)?;
        let bundle = record.encode_bundle().map_err(|_| Refusal::Failed)?;
        require_archivable(ledger, original)?;
        Ok(NativeTextJoinedSharedHistoryRecord {
            action: record.action,
            manifest: record.manifest.id(),
            bundle,
            _fee: fee,
        })
    }

    fn read_dependency(
        &self,
        channel: u128,
        action: ActionId,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<MemberSavedOriginal>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let mount = self
            .store
            .member_outbox_mount()
            .map_err(|_| Refusal::Unavailable)?;
        let key = member_outbox_key(self.store).map_err(|_| Refusal::Unavailable)?;
        let context = self
            .registration
            .member_outbox_context_data()
            .map_err(|_| Refusal::Failed)?;
        let saved = mount
            .lookup_existing_action(self.store, context, &key, action, rng)
            .map_err(|_| Refusal::Failed)?;
        if saved.as_ref().is_some_and(|saved| {
            saved.action().spec().realm.channel() != channel
                || saved.receipt().admission != self.registration.selected_data().1
        }) || !self
            .node
            .observe_member_publication(self.prefix)
            .is_ok_and(|original| original.is_current_cached())
        {
            return Err(Refusal::Stale);
        }
        Ok(saved)
    }
}

fn require_archivable(
    ledger: &NativeTextJoinedMemberLedger,
    original: &NativeTextJoinedReceivedOriginal,
) -> Result<(), V2MemberSaveRefusal> {
    if !ledger.is_current_cached()
        || !original.is_for_original_row_cached(&ledger.selection)
        || !ledger.ledger.matches_valid_ordinary_original(
            original.opened.action_data(),
            original.opened.admission_data(),
        )
    {
        return Err(V2MemberSaveRefusal::Stale);
    }
    Ok(())
}
