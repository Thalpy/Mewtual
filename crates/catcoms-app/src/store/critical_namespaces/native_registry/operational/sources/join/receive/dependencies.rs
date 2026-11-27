//! Connected catch-up of one exact ActionId. No method may dial or sign anew.
catcoms_rt::redacted_debug! {
    NativeTextJoinedArchiveDependency;
    NativeTextJoinedArchiveReplay;
    NativeTextJoinedDependencyExchange;
    NativeTextJoinedDependencyResponse;
    NativeTextJoinedDependencyProvider;
}

use super::history_discovery::{ArchivePageProof, HistoryContactScope};
use super::*;
use crate::store::persistence::v2_member_transport::{
    LocalHistoryDependencyReplay, HISTORY_REPLAY_FEE,
};
use catcoms_net::{
    NativeChatIngressAttachRefusal, NativeChatMemberDependencies,
    NativeChatMemberDependencyExchange, NativeChatMemberDependencyIncoming,
    NativeChatMemberDependencyObservation, NativeChatMemberDependencyRequests,
};
use catcoms_rt::TransportError;
use catcoms_sync::owner_v2::{
    OpenedV2ArchiveHistoryRequest, OpenedV2MemberDependencyRequest, OpenedV2MemberHistoryRequest,
    OpenedV2RemovedReviewRequest, PreparedV2MemberDependencyRequest, V2MemberDependencyRoute,
    V2MemberPublicationObservation, MAX_V2_DEPENDENCY_REPLY_BYTES,
};
use join_tcp::DependencyCaptureRefusal;

// Overlapping network buffers, nested sealed original and role-specific AEAD scratch.
pub(super) const REPLY_FEE: usize = OPEN_FEE + 5 * MAX_V2_DEPENDENCY_REPLY_BYTES + 16 * 1024;
const MAX_FRAME_DEPENDENCY_DEPTH: u8 = 4;

pub(super) enum DependencyProvenance {
    Direct,
    ApprovedAuthor(OwnerBodyId),
    ArchiveRelay,
    ArchivePage {
        proof: Arc<ArchivePageProof>,
        own_author: Option<OwnerBodyId>,
    },
}

impl DependencyProvenance {
    fn requires_holder_approval(&self) -> bool {
        matches!(self, Self::ArchiveRelay | Self::ArchivePage { .. })
    }

    fn uses_archive_wire(&self) -> bool {
        matches!(
            self,
            Self::ArchiveRelay
                | Self::ArchivePage {
                    own_author: None,
                    ..
                }
        )
    }

    fn bound_author(&self) -> Option<OwnerBodyId> {
        match self {
            Self::ApprovedAuthor(author) => Some(*author),
            Self::ArchivePage { own_author, .. } => *own_author,
            Self::Direct | Self::ArchiveRelay => None,
        }
    }
}

/// One fixed ActionId and original live route, ready outside the metadata slot.
pub struct NativeTextJoinedDependencyExchange {
    exchange: NativeChatMemberDependencyExchange,
    selected: NativeTextJoinedRowSelection,
    channel: u128,
    route: V2MemberDependencyRoute,
    request: PreparedV2MemberDependencyRequest,
    frame_depth: Option<u8>,
    provenance: DependencyProvenance,
    signal: RequestCancellation,
    invocation: watch::Sender<bool>,
}

/// One bounded reply on the exact original connection. Only a fresh mounted
/// capture can open it under the matching selected Core and MLS state.
pub struct NativeTextJoinedDependencyResponse {
    wire: bytes::Bytes,
    selected: NativeTextJoinedRowSelection,
    channel: u128,
    route: V2MemberDependencyRoute,
    request: PreparedV2MemberDependencyRequest,
    observation: NativeChatMemberDependencyObservation,
    frame_depth: Option<u8>,
    provenance: DependencyProvenance,
    signal: RequestCancellation,
    invocation: watch::Sender<bool>,
}

impl NativeTextJoinedDependencyResponse {
    fn require_original_selection(
        &self,
        selection: &NativeTextJoinedRowSelection,
    ) -> Result<(), V2MemberSaveRefusal> {
        if !selection.same_original(&self.selected) || !self.is_current_cached() {
            return Err(V2MemberSaveRefusal::Stale);
        }
        Ok(())
    }

    /// The exact response may wait for local staging while all of its
    /// original leases and observations remain live.
    pub fn is_current_cached(&self) -> bool {
        self.selected.is_current_cached()
            && self.observation.is_current_cached()
            && !self.signal.is_cancelled()
    }
}

/// One exact offered original and its paid staging ledger. It can yield
/// between local ancestor batches without retaining a physical row borrow.
pub struct NativeTextJoinedArchiveReplay {
    ledger: NativeTextJoinedMemberLedger,
    original: NativeTextJoinedReceivedOriginal,
    dependencies: Option<LocalHistoryDependencyReplay>,
    archive_page: Option<Arc<ArchivePageProof>>,
    _fee: CatalogReadReservation,
}

/// One exact missing ancestor derived from a checked original on a live
/// archive page. It retains no original, native invocation or wire payment.
#[derive(Clone)]
pub struct NativeTextJoinedArchiveDependency {
    pub(super) page: Arc<ArchivePageProof>,
    pub(super) parent: ActionId,
    child: ActionId,
    admission: [u8; 32],
    original: blake3::Hash,
}

impl NativeTextJoinedArchiveDependency {
    pub fn parent_data(&self) -> ActionId {
        self.parent
    }

    pub fn matches_child(&self, replay: &NativeTextJoinedArchiveReplay) -> bool {
        self.child == replay.original.action_id_data()
            && self.admission == *replay.original.opened.admission_data().as_bytes()
            && self.original == blake3::hash(replay.original.opened.original_data())
            && replay
                .archive_page
                .as_ref()
                .is_some_and(|page| Arc::ptr_eq(page, &self.page))
    }
}

impl NativeTextJoinedArchiveReplay {
    /// Only a checked original that found no selected local parent may
    /// issue this ticket. Refusals and traversal limits carry no ActionId.
    pub fn missing_archive_dependency(&self) -> Option<NativeTextJoinedArchiveDependency> {
        let page = self.archive_page.as_ref()?;
        let parent = self.dependencies.as_ref()?.exact_miss()?;
        if !self.ledger.is_current_cached()
            || !self
                .original
                .is_for_original_row_cached(&self.ledger.selection)
        {
            return None;
        }
        Some(NativeTextJoinedArchiveDependency {
            page: page.clone(),
            parent,
            child: self.original.action_id_data(),
            admission: *self.original.opened.admission_data().as_bytes(),
            original: blake3::hash(self.original.opened.original_data()),
        })
    }

    /// The Create target to invalidate after this exact write settles.
    pub fn target_data(&self) -> ActionId {
        self.original.projection_target_data()
    }

    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store::critical_namespaces::native_registry) fn original_digest_for_test(
        &self,
    ) -> blake3::Hash {
        self.original.original_digest_for_test()
    }
}

pub struct NativeTextJoinedDependencyProvider {
    _invocation: watch::Sender<bool>,
    requests: NativeChatMemberDependencyRequests,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTextJoinedDependencyAttachRefusal {
    Occupied,
    IngressContended,
    TransportContended,
    Member(V2MemberSaveRefusal),
}

impl From<DependencyCaptureRefusal> for V2MemberSaveRefusal {
    fn from(refusal: DependencyCaptureRefusal) -> Self {
        match refusal {
            DependencyCaptureRefusal::Contended => Self::Busy,
            DependencyCaptureRefusal::Unavailable => Self::Unavailable,
        }
    }
}

impl From<V2MemberSaveRefusal> for NativeTextJoinedDependencyAttachRefusal {
    fn from(refusal: V2MemberSaveRefusal) -> Self {
        Self::Member(refusal)
    }
}

impl NativeTextJoinedDependencyProvider {
    pub async fn next(&mut self) -> Option<NativeChatMemberDependencyIncoming> {
        self.requests.next().await
    }
}

impl NativeTextJoinedDependencyExchange {
    pub async fn receive(self) -> Result<NativeTextJoinedDependencyResponse, TransportError> {
        let observation = self.exchange.observe_route();
        let wire = self
            .exchange
            .request(self.request.wire_data(), self.signal.clone())
            .await?;
        if self.signal.is_cancelled() || !observation.is_current_cached() {
            return Err(TransportError::Cancelled);
        }
        Ok(NativeTextJoinedDependencyResponse {
            wire,
            selected: self.selected,
            channel: self.channel,
            route: self.route,
            request: self.request,
            observation,
            frame_depth: self.frame_depth,
            provenance: self.provenance,
            signal: self.signal,
            invocation: self.invocation,
        })
    }
}

#[derive(Debug)]
struct DependencyParentScope {
    caller: RequestCancellation,
    parent: RequestCancellation,
}

#[async_trait::async_trait]
impl RequestAuthority for DependencyParentScope {
    fn authorized(&self) -> bool {
        !self.caller.is_cancelled() && !self.parent.is_cancelled()
    }

    async fn revoked(&self) {
        let (mut caller, mut parent) = (self.caller.clone(), self.parent.clone());
        tokio::select! {
            _ = caller.cancelled() => {},
            _ = parent.cancelled() => {},
        }
    }
}

pub(super) struct ScopeAuthority(pub(super) RequestCancellation);
impl std::fmt::Debug for ScopeAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopeAuthority").finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl RequestAuthority for ScopeAuthority {
    fn authorized(&self) -> bool {
        !self.0.is_cancelled()
    }

    async fn revoked(&self) {
        self.0.clone().cancelled().await;
    }
}

pub(super) struct ReplyPayment {
    pub(super) _invocation: watch::Sender<bool>,
    pub(super) _fee: CatalogReadReservation,
}

impl std::fmt::Debug for ReplyPayment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplyPayment").finish_non_exhaustive()
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    pub fn prepare_dependency_exchange(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        provider: PeerId,
        action: ActionId,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedDependencyExchange, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let exchange = self
            .dependency_lane()?
            .capture_request(provider)
            .ok_or(Refusal::Unavailable)?;
        self.prepare_dependency_on(
            exchange,
            selection,
            (channel, action),
            (None, None, DependencyProvenance::Direct),
            caller,
            rng,
        )
    }

    /// Fetch a known exact ID from an approved connected archive holder. The
    /// returned original can belong to a different current author.
    pub fn prepare_archive_relay_exchange(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        provider: PeerId,
        action: ActionId,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedDependencyExchange, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let exchange = self
            .dependency_lane()?
            .capture_request(provider)
            .ok_or(Refusal::Unavailable)?;
        self.prepare_dependency_on(
            exchange,
            selection,
            (channel, action),
            (None, None, DependencyProvenance::ArchiveRelay),
            caller,
            rng,
        )
    }

    /// Recover a signed direct dependency of a frame or an authenticated
    /// reply on its original carrier. Every ancestor must stay current.
    pub fn prepare_received_dependency_exchange(
        self,
        original: &NativeTextJoinedReceivedOriginal,
        action: ActionId,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedDependencyExchange, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let selection = &original.selection;
        if !original.is_for_original_row_cached(selection) {
            return Err(Refusal::Stale);
        }
        let spec = original.opened.action_data().spec();
        if spec.parents.binary_search(&action).is_err() && spec.body.create() != Some(action) {
            return Err(Refusal::Failed);
        }
        let lane = self.dependency_lane()?;
        let (exchange, depth, parent) = match &original.origin {
            ReceiveOrigin::Gossip(frame) => (lane.capture_frame_request(frame), 1, None),
            ReceiveOrigin::Dependency {
                route,
                frame_depth: Some(depth),
                signal,
                ..
            } if *depth < MAX_FRAME_DEPENDENCY_DEPTH => {
                (lane.capture_frame_followup(route), depth + 1, Some(signal))
            }
            ReceiveOrigin::Dependency { .. } => return Err(Refusal::Unavailable),
        };
        self.prepare_dependency_on(
            exchange.ok_or(Refusal::Unavailable)?,
            selection,
            (spec.realm.channel(), action),
            (Some(depth), parent, DependencyProvenance::Direct),
            caller,
            rng,
        )
    }

    pub(super) fn prepare_dependency_on(
        self,
        exchange: NativeChatMemberDependencyExchange,
        selection: &NativeTextJoinedRowSelection,
        (channel, action): (u128, ActionId),
        (frame_depth, parent, provenance): (
            Option<u8>,
            Option<&RequestCancellation>,
            DependencyProvenance,
        ),
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedDependencyExchange, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let (requester, provider) = exchange.route_data();
        let route = V2MemberDependencyRoute {
            requester,
            provider,
        };
        let observation = exchange.observe_route();
        let (request, approval, observed, auth, fee) =
            self.with_member_receiver(selection, &caller, |source| {
                if !observation.is_current_cached() {
                    return Err(Refusal::Stale);
                }
                let approval = if provenance.requires_holder_approval() {
                    Some(source.require_history_peer(selection, provider)?.1)
                } else {
                    None
                };
                let fee = source
                    .metadata
                    .reserve_join_node(REPLY_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let request = if provenance.uses_archive_wire() {
                    source.node.prepare_member_archive_relay_request(
                        source.prefix,
                        channel,
                        route,
                        action,
                        rng,
                    )
                } else {
                    source.node.prepare_member_dependency_request(
                        source.prefix,
                        channel,
                        route,
                        action,
                        rng,
                    )
                }
                .map_err(|_| Refusal::Failed)?;
                let (observed, auth) = source.effect_data()?;
                Ok((request, approval, observed, auth, fee))
            })?;
        if !observation.is_current_cached() {
            return Err(Refusal::Stale);
        }
        let caller = if let Some(parent) = parent {
            if parent.is_cancelled() {
                return Err(Refusal::Stale);
            }
            caller
                .clone()
                .with_authority(Arc::new(DependencyParentScope {
                    caller,
                    parent: parent.clone(),
                }))
        } else {
            caller
        };
        let caller = if let Some(approval) = approval {
            HistoryContactScope::constrain(approval, caller)
        } else {
            caller
        };
        let (signal, invocation) = self.dependency_signal(selection, caller, auth, observed);
        let signal = signal.retain_custody(Arc::new(JoinFramePayment { _fee: fee }));
        Ok(NativeTextJoinedDependencyExchange {
            exchange,
            selected: selection.clone(),
            channel,
            route,
            request,
            frame_depth,
            provenance,
            signal,
            invocation,
        })
    }

    pub fn open_dependency_response(
        &self,
        selection: &NativeTextJoinedRowSelection,
        response: NativeTextJoinedDependencyResponse,
        caller: &RequestCancellation,
    ) -> Result<Option<NativeTextJoinedReceivedOriginal>, V2MemberSaveRefusal> {
        self.open_exact_dependency_reply(selection, &response, caller)
    }

    fn open_exact_dependency_reply(
        &self,
        selection: &NativeTextJoinedRowSelection,
        response: &NativeTextJoinedDependencyResponse,
        caller: &RequestCancellation,
    ) -> Result<Option<NativeTextJoinedReceivedOriginal>, V2MemberSaveRefusal> {
        response.require_original_selection(selection)?;
        let opened = self.with_member_receiver(selection, caller, |source| {
            source.open_dependency_reply(selection, response)
        })?;
        response.require_original_selection(selection)?;
        Ok(opened)
    }

    /// Open an offered original in a fresh paid staging ledger. Missing exact
    /// ancestors become a resumable traversal bound to the same original.
    pub fn prepare_discovered_archive(
        self,
        selection: &NativeTextJoinedRowSelection,
        response: NativeTextJoinedDependencyResponse,
        caller: RequestCancellation,
    ) -> Result<Option<NativeTextJoinedArchiveReplay>, V2MemberSaveRefusal> {
        self.prepare_parked_discovered_archive(selection, &response, caller)
    }

    /// Take one attempt at opening a retained paid response. The original
    /// route, invitation and signal stay bound to that response on retry.
    pub fn prepare_parked_discovered_archive(
        self,
        selection: &NativeTextJoinedRowSelection,
        response: &NativeTextJoinedDependencyResponse,
        caller: RequestCancellation,
    ) -> Result<Option<NativeTextJoinedArchiveReplay>, V2MemberSaveRefusal> {
        use catcoms_replication::chat::ActionState;
        use V2MemberSaveRefusal as Refusal;
        if matches!(&response.provenance, DependencyProvenance::Direct) {
            return Err(Refusal::Stale);
        }
        response.require_original_selection(selection)?;
        let channel = response.channel;
        let archive_page = match &response.provenance {
            DependencyProvenance::ArchivePage { proof, .. } => Some(proof.clone()),
            _ => None,
        };
        let require_current = |original: &NativeTextJoinedReceivedOriginal| {
            if caller.is_cancelled() || !original.is_for_original_row_cached(selection) {
                Err(Refusal::Stale)
            } else {
                Ok(())
            }
        };
        let Some((ledger, (original, dependencies, fee))) = self.prepare_member_ledger_with(
            selection,
            channel,
            (caller.clone(), ReceiverView::Live),
            |source| source.open_dependency_reply(selection, response),
            |source, ledger, view, original| {
                require_current(&original)?;
                let verifier = source
                    .prefix
                    .selected_member_verifier()
                    .map_err(|_| Refusal::Unavailable)?;
                let fee = source
                    .metadata
                    .reserve_join_node(HISTORY_REPLAY_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let retired = source
                    .prefix
                    .retired_history_data()
                    .iter()
                    .collect::<Vec<_>>();
                let status = original
                    .opened
                    .apply_with_local_ancestry(
                        (verifier, &retired),
                        (ledger, view),
                        RECEIVE_WORK,
                        false,
                    )
                    .map_err(|_| Refusal::Failed)?;
                let dependencies = match status {
                    V2OrdinaryReceiveStatus::WaitingForDependencies(missing) => {
                        Some(LocalHistoryDependencyReplay::new(missing)?)
                    }
                    V2OrdinaryReceiveStatus::Retained(ActionState::Valid) => None,
                    V2OrdinaryReceiveStatus::Retained(_) => return Err(Refusal::Failed),
                };
                Ok((original, dependencies, fee))
            },
        )?
        else {
            response.require_original_selection(selection)?;
            return Ok(None);
        };
        require_current(&original)?;
        Ok(Some(NativeTextJoinedArchiveReplay {
            ledger,
            original,
            dependencies,
            archive_page,
            _fee: fee,
        }))
    }

    /// Advance at most one bounded ancestor slice. A successful completion
    /// still needs the ordinary durable selector to publish its exact result.
    pub fn advance_discovered_archive(
        &self,
        replay: &mut NativeTextJoinedArchiveReplay,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<bool, V2MemberSaveRefusal> {
        use catcoms_replication::chat::ActionState;
        use V2MemberSaveRefusal as Refusal;
        let NativeTextJoinedArchiveReplay {
            ledger,
            original,
            dependencies,
            archive_page,
            ..
        } = replay;
        let selection = &ledger.selection;
        let signal = &ledger.signal;
        let require_current = || {
            if signal.is_cancelled()
                || caller.is_cancelled()
                || !original.is_for_original_row_cached(selection)
            {
                return Err(Refusal::Stale);
            }
            Ok(())
        };
        let Some(ancestors) = dependencies.as_mut() else {
            require_current()?;
            return Ok(true);
        };
        // Every call owns its own miss. An early mount or authority refusal
        // must never expose a previous traversal's result as a new ticket.
        ancestors.clear_exact_miss();
        require_current()?;
        let ready = self.with_member_receiver(selection, caller, |source| {
            if archive_page.is_some()
                && !source
                    .store
                    .has_mounted_shared_history()
                    .map_err(|_| Refusal::Busy)?
            {
                return Err(Refusal::Unavailable);
            }
            let verifier = source
                .prefix
                .selected_member_verifier()
                .map_err(|_| Refusal::Unavailable)?;
            let channel = ledger.ledger.realm().channel();
            let retired = source
                .prefix
                .retired_history_data()
                .iter()
                .collect::<Vec<_>>();
            if !ancestors.advance(
                (verifier, &retired, &mut ledger.ledger, &ledger.view),
                RECEIVE_WORK,
                (source.store, source.history_scope(channel)),
                |id| source.read_dependency(channel, id, rng),
                require_current,
            )? {
                return Ok(false);
            }
            let status = original
                .opened
                .apply_with_local_ancestry(
                    (verifier, &retired),
                    (&mut ledger.ledger, &ledger.view),
                    RECEIVE_WORK,
                    false,
                )
                .map_err(|_| Refusal::Failed)?;
            if status != V2OrdinaryReceiveStatus::Retained(ActionState::Valid) {
                return Err(Refusal::Unavailable);
            }
            require_current()?;
            Ok(true)
        })?;
        if ready {
            *dependencies = None;
        }
        Ok(ready)
    }

    /// Write the fully replayed original through the existing exact selector.
    pub fn select_discovered_archive(
        &self,
        replay: &NativeTextJoinedArchiveReplay,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<catcoms_storage::private_chat_page::PrivateChatPageId>, V2MemberSaveRefusal>
    {
        if replay.dependencies.is_some() {
            return Err(V2MemberSaveRefusal::Unavailable);
        }
        self.archive_received_member(&replay.ledger, &replay.original, caller, rng)
    }

    pub fn attach_dependency_provider(
        self,
        selection: &NativeTextJoinedRowSelection,
        caller: RequestCancellation,
    ) -> Result<NativeTextJoinedDependencyProvider, V2MemberSaveRefusal> {
        self.attach_dependency_provider_checked(selection, caller)
            .map_err(|refusal| match refusal {
                NativeTextJoinedDependencyAttachRefusal::Member(refusal) => refusal,
                _ => V2MemberSaveRefusal::Busy,
            })
    }

    pub fn attach_dependency_provider_checked(
        self,
        selection: &NativeTextJoinedRowSelection,
        caller: RequestCancellation,
    ) -> Result<NativeTextJoinedDependencyProvider, NativeTextJoinedDependencyAttachRefusal> {
        use NativeTextJoinedDependencyAttachRefusal as Attach;
        use V2MemberSaveRefusal as Refusal;
        let lane = self.dependency_lane().map_err(|refusal| match refusal {
            DependencyCaptureRefusal::Contended => Attach::TransportContended,
            DependencyCaptureRefusal::Unavailable => Attach::Member(Refusal::Unavailable),
        })?;
        let (observed, auth, fee) = self.with_member_receiver(selection, &caller, |source| {
            let fee = source
                .metadata
                .reserve_join_node(INGRESS_FEE)
                .map_err(|_| Refusal::Busy)?;
            let (observed, auth) = source.effect_data()?;
            Ok((observed, auth, fee))
        })?;
        let (signal, invocation) = self.dependency_signal(selection, caller, auth, observed);
        let signal = signal.retain_custody(Arc::new(JoinFramePayment { _fee: fee }));
        let requests = lane
            .try_attach_checked(Arc::new(ScopeAuthority(signal)))
            .map_err(|refusal| match refusal {
                NativeChatIngressAttachRefusal::Contended => Attach::IngressContended,
                NativeChatIngressAttachRefusal::Occupied => Attach::Occupied,
                NativeChatIngressAttachRefusal::Revoked => Attach::Member(Refusal::Cancelled),
                NativeChatIngressAttachRefusal::Closed => Attach::Member(Refusal::Unavailable),
            })?;
        Ok(NativeTextJoinedDependencyProvider {
            _invocation: invocation,
            requests,
        })
    }

    #[cfg(feature = "native-chat-test-hooks")]
    #[doc(hidden)]
    pub fn hold_dependency_ingress_for_test(self, hold: impl FnOnce()) -> Option<()> {
        let lane = self.dependency_lane().ok()?;
        // The network lock test must leave the metadata slot available.
        drop(self);
        lane.hold_ingress_for_test(hold)
    }

    #[cfg(feature = "native-chat-test-hooks")]
    #[doc(hidden)]
    pub fn hold_dependency_transport_for_test(self, hold: impl FnOnce()) -> Option<()> {
        let original = self.original.tcp.upgrade()?;
        // The TCP lock test must leave the metadata slot available.
        drop(self);
        original.hold_member_dependencies_for_test(hold)
    }

    pub fn serve_dependency_request(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        request: NativeChatMemberDependencyIncoming,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !self.dependency_lane()?.owns_request(&request) {
            return Err(Refusal::Stale);
        }
        let (_, wire) = request.current_data().ok_or(Refusal::Cancelled)?;
        if OpenedV2MemberHistoryRequest::recognizes(wire) {
            return self.serve_history_request(selection, channel, request, caller, rng);
        }
        if OpenedV2ArchiveHistoryRequest::recognizes(wire) {
            return self.serve_archive_history_request(selection, channel, request, caller, rng);
        }
        if OpenedV2RemovedReviewRequest::recognizes(wire) {
            return self.serve_removed_review_request(selection, channel, request, caller, rng);
        }
        let archive_relay = OpenedV2MemberDependencyRequest::recognizes_archive_relay(wire);
        let waiting = request.cancellation_authority();
        let (reply, approval, observed, auth, fee) =
            self.with_member_receiver(selection, &caller, |source| {
                if !waiting.authorized() {
                    return Err(Refusal::Cancelled);
                }
                let (requester, wire) = request.current_data().ok_or(Refusal::Cancelled)?;
                let route = V2MemberDependencyRoute {
                    requester,
                    provider: source.node.local_peer_data(),
                };
                let approval = if archive_relay {
                    Some(source.require_history_peer(selection, requester)?.1)
                } else {
                    None
                };
                let fee = source
                    .metadata
                    .reserve_join_node(REPLY_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let reply = if let Some(approval) = &approval {
                    let query = source
                        .node
                        .open_member_archive_relay_request(source.prefix, channel, route, wire)
                        .map_err(|_| Refusal::Failed)?;
                    if approval.is_cancelled() || !waiting.authorized() || caller.is_cancelled() {
                        return Err(Refusal::Cancelled);
                    }
                    source.serve_selected_archive_record(channel, route, &query, rng)?
                } else {
                    let original = source
                        .node
                        .open_member_dependency_request(source.prefix, channel, route, wire)
                        .map_err(|_| Refusal::Failed)?;
                    let dependency =
                        source.read_dependency(channel, original.action_id_data(), rng)?;
                    let saved = dependency
                        .as_ref()
                        .map(|saved| (saved.receipt().admission, saved.original_bytes()));
                    source
                        .node
                        .seal_member_dependency_reply(
                            source.prefix,
                            channel,
                            route,
                            &original,
                            saved,
                            rng,
                        )
                        .map_err(|_| Refusal::Failed)?
                };
                let (observed, auth) = source.effect_data()?;
                Ok((reply, approval, observed, auth, fee))
            })?;
        if !waiting.authorized() {
            return Err(Refusal::Cancelled);
        }
        let caller = if let Some(approval) = approval {
            HistoryContactScope::constrain(approval, caller)
        } else {
            caller
        };
        let (signal, invocation) = self.dependency_signal(selection, caller, auth, observed);
        request
            .respond(
                bytes::Bytes::from(reply),
                Arc::new(ScopeAuthority(signal)),
                Arc::new(ReplyPayment {
                    _invocation: invocation,
                    _fee: fee,
                }),
            )
            .map_err(|_| Refusal::Cancelled)
    }

    pub(super) fn dependency_lane(
        &self,
    ) -> Result<NativeChatMemberDependencies, DependencyCaptureRefusal> {
        self.original
            .tcp
            .upgrade()
            .ok_or(DependencyCaptureRefusal::Unavailable)?
            .capture_member_dependencies()
    }

    pub(super) fn dependency_signal(
        self,
        selection: &NativeTextJoinedRowSelection,
        caller: RequestCancellation,
        authentication: AuthenticationNegative,
        observed: V2MemberPublicationObservation,
    ) -> (RequestCancellation, watch::Sender<bool>) {
        let (invocation, live) = watch::channel(false);
        let authority = Arc::new(JoinEffect {
            scope: self.original.scope,
            ui: self.original.command.ui.into_effect_negative(),
            authentication,
            invocation: live,
            caller: caller.clone(),
            row: Some(selection.clone()),
        });
        (
            observed.constrain(caller.with_authority(authority)),
            invocation,
        )
    }
}

impl<R: CryptoRngCore> ReceiverSource<'_, R> {
    fn open_dependency_reply(
        &self,
        selection: &NativeTextJoinedRowSelection,
        response: &NativeTextJoinedDependencyResponse,
    ) -> Result<Option<NativeTextJoinedReceivedOriginal>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        response.require_original_selection(selection)?;
        let fee = self
            .metadata
            .reserve_join_node(OPEN_FEE)
            .map_err(|_| Refusal::Busy)?;
        let opened = self
            .node
            .open_member_dependency_reply(
                self.prefix,
                response.channel,
                response.route,
                &response.request,
                &response.wire,
            )
            .map_err(|_| Refusal::Failed)?;
        if response.provenance.bound_author().is_some_and(|author| {
            opened
                .as_ref()
                .is_some_and(|original| original.admission_data().as_bytes() != author.as_bytes())
        }) {
            return Err(Refusal::Stale);
        }
        let origin = ReceiveOrigin::Dependency {
            route: response.observation.clone(),
            frame_depth: response.frame_depth,
            signal: response.signal.clone(),
            _invocation: response.invocation.clone(),
        };
        if !origin.is_current_cached() {
            return Err(Refusal::Stale);
        }
        Ok(opened.map(|opened| NativeTextJoinedReceivedOriginal {
            opened,
            selection: selection.clone(),
            origin,
            _fee: fee,
        }))
    }

    fn serve_selected_archive_record(
        &self,
        channel: u128,
        route: V2MemberDependencyRoute,
        request: &OpenedV2MemberDependencyRequest,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<u8>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let selected = if self
            .store
            .has_mounted_shared_history()
            .map_err(|_| Refusal::Busy)?
        {
            self.store
                .read_selected_shared_history_record(
                    self.history_scope(channel),
                    request.action_id_data(),
                )
                .map_err(|_| Refusal::Failed)?
        } else {
            None
        };
        let saved = selected.as_ref().map(|record| {
            (
                record.admission,
                record.original.action(),
                record.original.original_bytes(),
            )
        });
        self.node
            .seal_member_archive_relay_reply(self.prefix, channel, route, request, saved, rng)
            .map_err(|_| Refusal::Failed)
    }

    pub(super) fn effect_data(
        &self,
    ) -> Result<(V2MemberPublicationObservation, AuthenticationNegative), V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let original = self
            .node
            .observe_member_publication(self.prefix)
            .map_err(|_| Refusal::Unavailable)?;
        let auth = self
            .metadata
            .authentication_negative()
            .map_err(|_| Refusal::Unavailable)?;
        Ok((original, auth))
    }
}
