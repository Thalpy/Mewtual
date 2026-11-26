//! Discover IDs in an approved connected member's own durable outbox.
//! Applying those originals still goes through exact dependency fetch and
//! the signed-action receiver. Cursor progress alone never covers a write.
use super::dependencies::{
    NativeTextJoinedDependencyExchange, ReplyPayment, ScopeAuthority, REPLY_FEE,
};
use super::*;
use crate::store::persistence::v2_member_outbox::{DispositionCode, MemberEntryStatus};
use catcoms_net::{
    NativeChatMemberDependencyExchange, NativeChatMemberDependencyIncoming,
    NativeChatMemberDependencyObservation,
};
use catcoms_rt::{ConnectionLease, TransportError};
use catcoms_sync::owner_v2::{
    PreparedV2MemberHistoryRequest, V2MemberHistoryCursor, V2MemberHistoryEntry,
    V2MemberHistoryPage, V2_MEMBER_HISTORY_PAGE_SPAN,
};

catcoms_rt::redacted_debug! {
    NativeTextJoinedHistoryExchange;
    NativeTextJoinedHistoryResponse;
    NativeTextJoinedHistoryPage;
}

pub struct NativeTextJoinedHistoryExchange {
    transport: NativeChatMemberDependencyExchange,
    request: PreparedV2MemberHistoryRequest,
    source: HistoryQuery,
}

struct HistoryQuery {
    row: NativeTextJoinedRowSelection,
    peer: PeerId,
    admission: OwnerBodyId,
    channel: u128,
    observation: NativeChatMemberDependencyObservation,
    connection: ConnectionLease,
    signal: RequestCancellation,
    _invocation: watch::Sender<bool>,
}

catcoms_rt::redacted_debug! { HistoryContactScope; }

struct HistoryContactScope {
    approval: RequestCancellation,
    caller: RequestCancellation,
}

impl HistoryContactScope {
    fn constrain(
        approval: RequestCancellation,
        caller: RequestCancellation,
    ) -> RequestCancellation {
        caller
            .clone()
            .with_authority(Arc::new(Self { approval, caller }))
    }
}

#[async_trait::async_trait]
impl RequestAuthority for HistoryContactScope {
    fn authorized(&self) -> bool {
        !self.caller.is_cancelled() && !self.approval.is_cancelled()
    }

    async fn revoked(&self) {
        let (mut caller, mut approval) = (self.caller.clone(), self.approval.clone());
        tokio::select! {
            _ = caller.cancelled() => {},
            _ = approval.cancelled() => {},
        }
    }
}

impl HistoryQuery {
    fn is_current(&self, selection: &NativeTextJoinedRowSelection) -> bool {
        selection.same_original(&self.row)
            && selection.is_current_cached()
            && self.observation.is_current_cached()
            && !self.signal.is_cancelled()
    }
}

/// One paid reply. Opening it requires the actual original native row again.
pub struct NativeTextJoinedHistoryResponse {
    wire: bytes::Bytes,
    request: PreparedV2MemberHistoryRequest,
    source: HistoryQuery,
}

/// Bounded hints from the original request. The caller keeps the same
/// connection and row live while turning any ID into an exact fetch.
pub struct NativeTextJoinedHistoryPage {
    page: V2MemberHistoryPage,
    query: HistoryQuery,
}

impl NativeTextJoinedHistoryPage {
    pub fn data(&self) -> Option<&V2MemberHistoryPage> {
        self.query.is_current(&self.query.row).then_some(&self.page)
    }
}

impl NativeTextJoinedHistoryExchange {
    pub async fn receive(self) -> Result<NativeTextJoinedHistoryResponse, TransportError> {
        let wire = self
            .transport
            .request(self.request.wire_data(), self.source.signal.clone())
            .await?;
        if !self.source.is_current(&self.source.row) {
            return Err(TransportError::Cancelled);
        }
        Ok(NativeTextJoinedHistoryResponse {
            wire,
            request: self.request,
            source: self.source,
        })
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    /// Request one page from a previously approved signed contact. This
    /// captures an existing connection only and does not initiate dialing.
    pub fn prepare_history_exchange(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        provider: PeerId,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedHistoryExchange, V2MemberSaveRefusal> {
        self.prepare_history_page(
            selection,
            (channel, provider, V2MemberHistoryCursor::first()),
            None,
            caller,
            rng,
        )
    }

    /// Continue a sweep on its original connection and row after dealing with
    /// each offered ID. A dropped sweep can restart from the first page.
    pub fn prepare_history_continuation(
        self,
        selection: &NativeTextJoinedRowSelection,
        page: &NativeTextJoinedHistoryPage,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<NativeTextJoinedHistoryExchange>, V2MemberSaveRefusal> {
        if !page.query.is_current(selection) {
            return Err(V2MemberSaveRefusal::Stale);
        }
        let Some(cursor) = page.page.continuation() else {
            return Ok(None);
        };
        self.prepare_history_page(
            selection,
            (page.query.channel, page.query.peer, cursor),
            Some(&page.query),
            caller,
            rng,
        )
        .map(Some)
    }

    fn prepare_history_page(
        self,
        selection: &NativeTextJoinedRowSelection,
        (channel, provider, cursor): (u128, PeerId, V2MemberHistoryCursor),
        previous: Option<&HistoryQuery>,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedHistoryExchange, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let transport = self
            .dependency_lane()?
            .capture_request(provider)
            .ok_or(Refusal::Unavailable)?;
        let observation = transport.observe_route();
        let connection = transport.observe_connection();
        let (request, admission, approval, observed, auth, fee) =
            self.with_member_receiver(selection, &caller, |source| {
                if !observation.is_current_cached() {
                    return Err(Refusal::Stale);
                }
                let (admission, approval) = source.require_history_peer(selection, provider)?;
                if previous.is_some_and(|previous| {
                    !previous.is_current(selection)
                        || previous.admission != admission
                        || !connection.is_same(&previous.connection)
                }) {
                    return Err(Refusal::Stale);
                }
                // The underlying dependency lane keeps its original reply
                // bound. Account for that overlap even for a small page.
                let fee = source
                    .metadata
                    .reserve_join_node(REPLY_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let request = source
                    .node
                    .member_history_access(source.prefix, channel)
                    .and_then(|access| access.prepare_request(provider, cursor, rng))
                    .map_err(|_| Refusal::Failed)?;
                let (observed, auth) = source.effect_data()?;
                Ok((request, admission, approval, observed, auth, fee))
            })?;
        let caller = HistoryContactScope::constrain(approval, caller);
        let (signal, invocation) = self.dependency_signal(selection, caller, auth, observed);
        let signal = signal.retain_custody(Arc::new(JoinFramePayment { _fee: fee }));
        Ok(NativeTextJoinedHistoryExchange {
            transport,
            request,
            source: HistoryQuery {
                row: selection.clone(),
                peer: provider,
                admission,
                channel,
                observation,
                connection,
                signal,
                _invocation: invocation,
            },
        })
    }

    /// Fetch one ID offered on this live page using the frozen connection.
    /// An exact reply is still subject to Core and durable receiver checks.
    pub fn prepare_discovered_original(
        self,
        selection: &NativeTextJoinedRowSelection,
        page: &NativeTextJoinedHistoryPage,
        action: ActionId,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedDependencyExchange, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !page.query.is_current(selection)
            || !page
                .page
                .entries()
                .iter()
                .any(|entry| entry.action == action)
        {
            return Err(Refusal::Stale);
        }
        let exchange = self
            .dependency_lane()?
            .capture_request(page.query.peer)
            .ok_or(Refusal::Unavailable)?;
        if !exchange
            .observe_connection()
            .is_same(&page.query.connection)
        {
            return Err(Refusal::Stale);
        }
        self.prepare_dependency_on(
            exchange,
            selection,
            (page.query.channel, action),
            (None, Some(&page.query.signal), Some(page.query.admission)),
            caller,
            rng,
        )
    }

    pub fn open_history_response(
        self,
        selection: &NativeTextJoinedRowSelection,
        response: NativeTextJoinedHistoryResponse,
        caller: &RequestCancellation,
    ) -> Result<NativeTextJoinedHistoryPage, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let NativeTextJoinedHistoryResponse {
            wire,
            request,
            source: query,
        } = response;
        if !query.is_current(selection) {
            return Err(Refusal::Stale);
        }
        let page = self.with_member_receiver(selection, caller, |source| {
            if source.require_history_peer(selection, query.peer)?.0 != query.admission {
                return Err(Refusal::Stale);
            }
            source
                .node
                .member_history_access(source.prefix, query.channel)
                .and_then(|access| access.open_reply(query.peer, &request, &wire))
                .map_err(|_| Refusal::Failed)
        })?;
        if !query.is_current(selection) {
            return Err(Refusal::Stale);
        }
        Ok(NativeTextJoinedHistoryPage { page, query })
    }

    pub(super) fn serve_history_request(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        request: NativeChatMemberDependencyIncoming,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let waiting = request.cancellation_authority();
        let (reply, approval, observed, auth, fee) =
            self.with_member_receiver(selection, &caller, |source| {
                let (requester, wire) = request.current_data().ok_or(Refusal::Cancelled)?;
                let (_, approval) = source.require_history_peer(selection, requester)?;
                let fee = source
                    .metadata
                    .reserve_join_node(REPLY_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let access = source
                    .node
                    .member_history_access(source.prefix, channel)
                    .map_err(|_| Refusal::Unavailable)?;
                let original = access
                    .open_request(requester, wire)
                    .map_err(|_| Refusal::Failed)?;
                if !waiting.authorized() {
                    return Err(Refusal::Cancelled);
                }
                let page = source.read_history_page(selection, channel, original.cursor())?;
                let reply = access
                    .seal_reply(requester, &original, &page, rng)
                    .map_err(|_| Refusal::Failed)?;
                let (observed, auth) = source.effect_data()?;
                Ok((reply, approval, observed, auth, fee))
            })?;
        if !waiting.authorized() {
            return Err(Refusal::Cancelled);
        }
        let caller = HistoryContactScope::constrain(approval, caller);
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
}

impl<R: CryptoRngCore> ReceiverSource<'_, R> {
    fn require_history_peer(
        &self,
        selection: &NativeTextJoinedRowSelection,
        peer: PeerId,
    ) -> Result<(OwnerBodyId, RequestCancellation), V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let replay = self
            .prefix
            .selected_member_verifier()
            .map_err(|_| Refusal::Unavailable)?
            .replay_data();
        selection
            .approved_history_peer(peer, replay)
            .ok_or(Refusal::Stale)
    }

    fn read_history_page(
        &self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        cursor: V2MemberHistoryCursor,
    ) -> Result<V2MemberHistoryPage, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let mount = self
            .store
            .installed_member_outbox_mount()
            .map_err(|_| Refusal::Failed)?
            .ok_or(Refusal::Busy)?;
        let key = member_outbox_key(self.store).map_err(|_| Refusal::Unavailable)?;
        let page = mount
            .read_selected_history_page(
                self.store,
                selection.original.member_outbox,
                &key,
                cursor.bounds(),
                V2_MEMBER_HISTORY_PAGE_SPAN,
            )
            .map_err(|error| V2MemberSaveRefusal::from(&error))?;
        let admission = self.registration.selected_data().1;
        let entries = page
            .items()
            .iter()
            .filter(|item| {
                item.receipt().admission == admission
                    && item.action().spec().realm.channel() == channel
                    && item.status() != MemberEntryStatus::Settled(DispositionCode::Held)
            })
            .map(|item| V2MemberHistoryEntry {
                ordinal: item.receipt().ordinal,
                action: item.receipt().action,
            })
            .collect();
        V2MemberHistoryPage::new(cursor, page.end_exclusive(), entries).map_err(|_| Refusal::Failed)
    }
}
