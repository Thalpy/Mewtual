//! Discover IDs in an approved connected member's own durable outbox.
//! Applying those originals still goes through exact dependency fetch and
//! the signed-action receiver. Cursor progress alone never covers a write.
use super::dependencies::{
    DependencyProvenance, NativeTextJoinedDependencyExchange, ReplyPayment, ScopeAuthority,
    REPLY_FEE,
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

// A page can wait between turns. Pay for its bounded IDs, approval, and
// row/lease observations without retaining the departed wire buffers.
const PARKED_PAGE_FEE: usize = 8 * 1024;

pub struct NativeTextJoinedHistoryExchange {
    transport: NativeChatMemberDependencyExchange,
    request: PreparedV2MemberHistoryRequest,
    source: HistoryQuery,
    wire_scope: HistoryWireScope,
}

struct HistoryQuery {
    row: NativeTextJoinedRowSelection,
    peer: PeerId,
    admission: OwnerBodyId,
    channel: u128,
    observation: NativeChatMemberDependencyObservation,
    connection: ConnectionLease,
    signal: RequestCancellation,
    _fee: CatalogReadReservation,
}

// Only a live request/response holds a captured native command and the
// maximum wire reservation. Parked pages keep row and selection observations.
struct HistoryWireScope {
    signal: RequestCancellation,
    _invocation: watch::Sender<bool>,
}

catcoms_rt::redacted_debug! { HistoryContactScope; }

pub(super) struct HistoryContactScope {
    approval: RequestCancellation,
    caller: RequestCancellation,
}

impl HistoryContactScope {
    pub(super) fn constrain(
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
    wire_scope: HistoryWireScope,
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
            .request(self.request.wire_data(), self.wire_scope.signal.clone())
            .await?;
        if self.wire_scope.signal.is_cancelled() || !self.source.is_current(&self.source.row) {
            return Err(TransportError::Cancelled);
        }
        Ok(NativeTextJoinedHistoryResponse {
            wire,
            request: self.request,
            source: self.source,
            wire_scope: self.wire_scope,
        })
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    /// Select an already authenticated archive entry before sending an exact
    /// fetch. A successful lookup supplies its Create target for UI invalidation.
    pub fn selected_discovered_target(
        &self,
        selection: &NativeTextJoinedRowSelection,
        page: &NativeTextJoinedHistoryPage,
        action: ActionId,
        caller: &RequestCancellation,
    ) -> Result<Option<ActionId>, V2MemberSaveRefusal> {
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
        let selected = self.with_member_receiver(selection, caller, |source| {
            if !source
                .store
                .has_mounted_shared_history()
                .map_err(|_| Refusal::Busy)?
            {
                return Err(Refusal::Unavailable);
            }
            let record = source
                .store
                .read_shared_history_record(source.history_scope(page.query.channel), action)
                .map_err(|_| Refusal::Failed)?;
            let Some(record) = record else {
                return Ok(None);
            };
            if record.admission.as_bytes() != page.query.admission.as_bytes() {
                return Err(Refusal::Stale);
            }
            Ok(Some(
                record
                    .original
                    .action()
                    .spec()
                    .body
                    .create()
                    .unwrap_or(action),
            ))
        })?;
        if !page.query.is_current(selection) {
            return Err(Refusal::Stale);
        }
        Ok(selected)
    }

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
        let (request, admission, approval, observed, parked, auth, wire_fee, page_fee) = self
            .with_member_receiver(selection, &caller, |source| {
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
                let wire_fee = source
                    .metadata
                    .reserve_join_node(REPLY_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let page_fee = source
                    .metadata
                    .reserve_join_node(PARKED_PAGE_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let request = source
                    .node
                    .member_history_access(source.prefix, channel)
                    .and_then(|access| access.prepare_request(provider, cursor, rng))
                    .map_err(|_| Refusal::Failed)?;
                let (observed, auth) = source.effect_data()?;
                let parked = source
                    .node
                    .observe_member_publication(source.prefix)
                    .map_err(|_| Refusal::Stale)?;
                Ok((
                    request, admission, approval, observed, parked, auth, wire_fee, page_fee,
                ))
            })?;
        let caller = HistoryContactScope::constrain(approval, caller);
        let (wire, invocation) = self.dependency_signal(selection, caller.clone(), auth, observed);
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
                signal: parked.constrain(caller),
                _fee: page_fee,
            },
            wire_scope: HistoryWireScope {
                signal: wire.retain_custody(Arc::new(JoinFramePayment { _fee: wire_fee })),
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
            (
                None,
                Some(&page.query.signal),
                DependencyProvenance::ApprovedAuthor(page.query.admission),
            ),
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
        let query = &response.source;
        if response.wire_scope.signal.is_cancelled() || !query.is_current(selection) {
            return Err(Refusal::Stale);
        }
        let page = self.with_member_receiver(selection, caller, |source| {
            if source.require_history_peer(selection, query.peer)?.0 != query.admission {
                return Err(Refusal::Stale);
            }
            source
                .node
                .member_history_access(source.prefix, query.channel)
                .and_then(|access| access.open_reply(query.peer, &response.request, &response.wire))
                .map_err(|_| Refusal::Failed)
        })?;
        if response.wire_scope.signal.is_cancelled() || !query.is_current(selection) {
            return Err(Refusal::Stale);
        }
        // The response drops its paid wire before refunding that payment.
        Ok(NativeTextJoinedHistoryPage {
            page,
            query: response.source,
        })
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
    pub(super) fn require_history_peer(
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
