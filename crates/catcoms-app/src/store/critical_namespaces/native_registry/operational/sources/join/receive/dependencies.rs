//! Connected catch-up of one exact ActionId. No method may dial or sign anew.
catcoms_rt::redacted_debug! {
    NativeTextJoinedDependencyExchange;
    NativeTextJoinedDependencyResponse;
    NativeTextJoinedDependencyProvider;
}

use super::*;
use catcoms_net::{
    NativeChatIngressAttachRefusal, NativeChatMemberDependencies,
    NativeChatMemberDependencyExchange, NativeChatMemberDependencyIncoming,
    NativeChatMemberDependencyObservation, NativeChatMemberDependencyRequests,
};
use catcoms_rt::TransportError;
use catcoms_sync::owner_v2::{
    PreparedV2MemberDependencyRequest, V2MemberDependencyRoute, V2MemberPublicationObservation,
    MAX_V2_DEPENDENCY_REPLY_BYTES,
};

// Overlapping network buffers, nested sealed original and role-specific AEAD scratch.
const REPLY_FEE: usize = OPEN_FEE + 5 * MAX_V2_DEPENDENCY_REPLY_BYTES + 16 * 1024;
const MAX_FRAME_DEPENDENCY_DEPTH: u8 = 4;

/// One fixed ActionId and original live route, ready outside the metadata slot.
pub struct NativeTextJoinedDependencyExchange {
    exchange: NativeChatMemberDependencyExchange,
    selected: NativeTextJoinedRowSelection,
    channel: u128,
    route: V2MemberDependencyRoute,
    request: PreparedV2MemberDependencyRequest,
    frame_depth: Option<u8>,
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
    signal: RequestCancellation,
    invocation: watch::Sender<bool>,
}

pub struct NativeTextJoinedDependencyProvider {
    _invocation: watch::Sender<bool>,
    requests: NativeChatMemberDependencyRequests,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTextJoinedDependencyAttachRefusal {
    Occupied,
    IngressContended,
    Member(V2MemberSaveRefusal),
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
            (None, None),
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
            (Some(depth), parent),
            caller,
            rng,
        )
    }

    fn prepare_dependency_on(
        self,
        exchange: NativeChatMemberDependencyExchange,
        selection: &NativeTextJoinedRowSelection,
        (channel, action): (u128, ActionId),
        (frame_depth, parent): (Option<u8>, Option<&RequestCancellation>),
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
        let (request, observed, auth, fee) =
            self.with_member_receiver(selection, &caller, |source| {
                if !observation.is_current_cached() {
                    return Err(Refusal::Stale);
                }
                let fee = source
                    .metadata
                    .reserve_join_node(REPLY_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let request = source
                    .node
                    .prepare_member_dependency_request(source.prefix, channel, route, action, rng)
                    .map_err(|_| Refusal::Failed)?;
                let (observed, auth) = source.effect_data()?;
                Ok((request, observed, auth, fee))
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
        let (signal, invocation) = self.dependency_signal(selection, caller, auth, observed);
        let signal = signal.retain_custody(Arc::new(JoinFramePayment { _fee: fee }));
        Ok(NativeTextJoinedDependencyExchange {
            exchange,
            selected: selection.clone(),
            channel,
            route,
            request,
            frame_depth,
            signal,
            invocation,
        })
    }

    pub fn open_dependency_response(
        self,
        selection: &NativeTextJoinedRowSelection,
        response: NativeTextJoinedDependencyResponse,
        caller: &RequestCancellation,
    ) -> Result<Option<NativeTextJoinedReceivedOriginal>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !selection.same_original(&response.selected)
            || response.signal.is_cancelled()
            || !response.observation.is_current_cached()
        {
            return Err(Refusal::Stale);
        }
        self.with_member_receiver(selection, caller, |source| {
            let response = response;
            let fee = source
                .metadata
                .reserve_join_node(OPEN_FEE)
                .map_err(|_| Refusal::Busy)?;
            let opened = source
                .node
                .open_member_dependency_reply(
                    source.prefix,
                    response.channel,
                    response.route,
                    &response.request,
                    &response.wire,
                )
                .map_err(|_| Refusal::Failed)?;
            let NativeTextJoinedDependencyResponse {
                wire,
                observation,
                frame_depth,
                signal,
                invocation,
                ..
            } = response;
            drop(wire);
            let origin = ReceiveOrigin::Dependency {
                route: observation,
                frame_depth,
                signal,
                _invocation: invocation,
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
        })
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
        let lane = self.dependency_lane()?;
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
        let waiting = request.cancellation_authority();
        let (reply, observed, auth, fee) =
            self.with_member_receiver(selection, &caller, |source| {
                if !waiting.authorized() {
                    return Err(Refusal::Cancelled);
                }
                let (requester, wire) = request.current_data().ok_or(Refusal::Cancelled)?;
                let route = V2MemberDependencyRoute {
                    requester,
                    provider: source.node.local_peer_data(),
                };
                let fee = source
                    .metadata
                    .reserve_join_node(REPLY_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let original = source
                    .node
                    .open_member_dependency_request(source.prefix, channel, route, wire)
                    .map_err(|_| Refusal::Failed)?;
                let dependency = source.read_dependency(channel, original.action_id_data(), rng)?;
                let saved = dependency
                    .as_ref()
                    .map(|saved| (saved.receipt().admission, saved.original_bytes()));
                let reply = source
                    .node
                    .seal_member_dependency_reply(
                        source.prefix,
                        channel,
                        route,
                        &original,
                        saved,
                        rng,
                    )
                    .map_err(|_| Refusal::Failed)?;
                let (observed, auth) = source.effect_data()?;
                Ok((reply, observed, auth, fee))
            })?;
        if !waiting.authorized() {
            return Err(Refusal::Cancelled);
        }
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

    fn dependency_lane(&self) -> Result<NativeChatMemberDependencies, V2MemberSaveRefusal> {
        self.original
            .tcp
            .upgrade()
            .ok_or(V2MemberSaveRefusal::Unavailable)?
            .capture_member_dependencies()
            .map_err(|_| V2MemberSaveRefusal::Unavailable)
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
