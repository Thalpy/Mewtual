//! Discover IDs in an approved connected member's own durable outbox.
//! Applying those originals still goes through exact dependency fetch and
//! the signed-action receiver. Cursor progress alone never covers a write.
use super::dependencies::{
    DependencyProvenance, NativeTextJoinedArchiveDependency, NativeTextJoinedDependencyExchange,
    ReplyPayment, ScopeAuthority, REPLY_FEE,
};
use super::*;
use crate::store::persistence::v2_member_outbox::{DispositionCode, MemberEntryStatus};
use catcoms_net::{
    NativeChatMemberDependencyIncoming, NativeChatMemberDependencyObservation,
    NativeChatMemberInventoryExchange,
};
use catcoms_replication::chat::Body;
use catcoms_rt::{ConnectionLease, TransportError};
use catcoms_sync::owner_v2::{
    PreparedV2ArchiveHistoryRequest, PreparedV2MemberHistoryRequest, V2ArchiveHistoryCursor,
    V2ArchiveHistoryPage, V2MemberHistoryCursor, V2MemberHistoryEntry, V2MemberHistoryPage,
    V2_MEMBER_HISTORY_PAGE_SPAN,
};

catcoms_rt::redacted_debug! {
    NativeTextJoinedHistoryExchange;
    NativeTextJoinedHistoryResponse;
    NativeTextJoinedHistoryPage;
}

// A page can wait between turns. Pay for its bounded IDs, approval, and
// row/lease observations without retaining the departed wire buffers.
const PARKED_PAGE_FEE: usize = 8 * 1024;
// Archive pages also pay for the worker's bounded stack of exact signed
// identities while it drops a child's paid ledger to ask for a parent.
const ARCHIVE_ANCESTRY_FEE: usize = 4 * 1024;

pub struct NativeTextJoinedHistoryExchange {
    transport: NativeChatMemberInventoryExchange,
    request: HistoryRequest,
    source: HistoryQuery,
    wire_scope: HistoryWireScope,
}

enum HistoryCursor {
    Own(V2MemberHistoryCursor),
    Archive {
        cursor: Option<V2ArchiveHistoryCursor>,
        after: Option<ActionId>,
    },
}

enum HistoryRequest {
    Own(PreparedV2MemberHistoryRequest),
    Archive(PreparedV2ArchiveHistoryRequest, Option<ActionId>),
}

impl HistoryRequest {
    fn wire_data(&self) -> &[u8] {
        match self {
            Self::Own(request) => request.wire_data(),
            Self::Archive(request, _) => request.wire_data(),
        }
    }
}

enum HistoryPage {
    Own(V2MemberHistoryPage),
    Archive {
        page: V2ArchiveHistoryPage,
        highest: Option<ActionId>,
        proof: Arc<ArchivePageProof>,
    },
}

/// Identity of one opened archive page. Ancestry tickets retain this
/// identity without retaining wire buffers or an active native command.
pub(super) struct ArchivePageProof(());

impl HistoryPage {
    fn contains(&self, action: ActionId) -> bool {
        match self {
            Self::Own(page) => page.entries().iter().any(|entry| entry.action == action),
            Self::Archive { page, .. } => page.actions().binary_search(&action).is_ok(),
        }
    }

    fn continuation(&self) -> Option<HistoryCursor> {
        match self {
            Self::Own(page) => page.continuation().map(HistoryCursor::Own),
            Self::Archive { page, highest, .. } => Some(HistoryCursor::Archive {
                cursor: Some(page.continuation()?),
                after: *highest,
            }),
        }
    }
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
    request: HistoryRequest,
    source: HistoryQuery,
    wire_scope: HistoryWireScope,
}

/// Bounded hints from the original request. The caller keeps the same
/// connection and row live while turning any ID into an exact fetch.
pub struct NativeTextJoinedHistoryPage {
    page: HistoryPage,
    query: HistoryQuery,
}

impl NativeTextJoinedHistoryPage {
    pub fn data(&self) -> Option<&V2MemberHistoryPage> {
        match &self.page {
            HistoryPage::Own(page) if self.is_current_cached() => Some(page),
            _ => None,
        }
    }

    pub fn is_current_cached(&self) -> bool {
        self.query.is_current(&self.query.row)
    }

    #[cfg(feature = "native-chat-test-hooks")]
    #[doc(hidden)]
    pub fn current_checks_for_test(&self) -> (bool, bool, bool) {
        (
            self.query.row.is_current_cached(),
            self.query.observation.is_current_cached(),
            !self.query.signal.is_cancelled(),
        )
    }

    pub fn action_at(&self, index: usize) -> Option<ActionId> {
        if !self.is_current_cached() {
            return None;
        }
        match &self.page {
            HistoryPage::Own(page) => page.entries().get(index).map(|entry| entry.action),
            HistoryPage::Archive { page, .. } => page.actions().get(index).copied(),
        }
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
        if !page.query.is_current(selection) || !page.page.contains(action) {
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
            match page.page {
                HistoryPage::Own(_)
                    if record.admission.as_bytes() != page.query.admission.as_bytes() =>
                {
                    return Err(Refusal::Stale);
                }
                HistoryPage::Archive { .. } => {
                    let verifier = source
                        .prefix
                        .selected_member_verifier()
                        .map_err(|_| Refusal::Stale)?;
                    verifier
                        .observe_live_member(record.admission)
                        .map_err(|_| Refusal::Stale)?;
                    verifier
                        .admission(record.admission)
                        .and_then(|author| {
                            author
                                .admission_data()
                                .matches_action_delegation(record.original.action())
                        })
                        .map_err(|_| Refusal::Stale)?;
                }
                HistoryPage::Own(_) => {}
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

    /// A clean own outbox can cover an offered archive Create for this sweep.
    /// The semantic ID proves local custody of the operation. Archive custody,
    /// the holder's signature bytes, and delivery remain independent.
    pub fn covers_discovered_own_create(
        &self,
        selection: &NativeTextJoinedRowSelection,
        page: &NativeTextJoinedHistoryPage,
        action: ActionId,
        caller: &RequestCancellation,
    ) -> Result<bool, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !page.query.is_current(selection) || !page.page.contains(action) {
            return Err(Refusal::Stale);
        }
        if !matches!(&page.page, HistoryPage::Archive { .. }) {
            return Ok(false);
        }
        let covered = self.with_member_receiver(selection, caller, |source| {
            let Some(saved) = source.read_selected_own_history_action(selection, action) else {
                return Ok(false);
            };
            let admission = source.registration.selected_data().1;
            if saved.receipt().admission != admission
                || saved.action().spec().realm.channel() != page.query.channel
                || !matches!(saved.action().spec().body, Body::Create { .. })
                || saved.status() == MemberEntryStatus::Settled(DispositionCode::Held)
            {
                return Ok(false);
            }
            let verifier = source
                .prefix
                .selected_member_verifier()
                .map_err(|_| Refusal::Stale)?;
            verifier
                .observe_live_member(admission)
                .map_err(|_| Refusal::Stale)?;
            Ok(verifier
                .admission(admission)
                .and_then(|author| {
                    author
                        .admission_data()
                        .matches_action_delegation(saved.action())
                })
                .is_ok())
        })?;
        if !page.query.is_current(selection) {
            return Err(Refusal::Stale);
        }
        Ok(covered)
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
            (
                channel,
                provider,
                HistoryCursor::Own(V2MemberHistoryCursor::first()),
            ),
            None,
            caller,
            rng,
        )
    }

    /// Discover eligible originals in a connected holder's shared archive.
    /// The separate own-outbox sweep retains its original wire contract.
    pub fn prepare_archive_history_exchange(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        provider: PeerId,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedHistoryExchange, V2MemberSaveRefusal> {
        self.prepare_history_page(
            selection,
            (
                channel,
                provider,
                HistoryCursor::Archive {
                    cursor: None,
                    after: None,
                },
            ),
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
        (channel, provider, cursor): (u128, PeerId, HistoryCursor),
        previous: Option<&HistoryQuery>,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedHistoryExchange, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let transport = self
            .dependency_lane()?
            .capture_inventory(provider)
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
                let ancestry_fee = if matches!(&cursor, HistoryCursor::Archive { .. }) {
                    ARCHIVE_ANCESTRY_FEE
                } else {
                    0
                };
                let page_fee = source
                    .metadata
                    .reserve_join_node(PARKED_PAGE_FEE + ancestry_fee)
                    .map_err(|_| Refusal::Busy)?;
                let access = source
                    .node
                    .member_history_access(source.prefix, channel)
                    .map_err(|_| Refusal::Failed)?;
                let request = match cursor {
                    HistoryCursor::Own(cursor) => access
                        .prepare_request(provider, cursor, rng)
                        .map(HistoryRequest::Own),
                    HistoryCursor::Archive { cursor, after } => access
                        .prepare_archive_page(provider, cursor, rng)
                        .map(|request| HistoryRequest::Archive(request, after)),
                }
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
        if !page.query.is_current(selection) || !page.page.contains(action) {
            return Err(Refusal::Stale);
        }
        let provenance = match &page.page {
            HistoryPage::Own(_) => DependencyProvenance::ApprovedAuthor(page.query.admission),
            HistoryPage::Archive { proof, .. } => DependencyProvenance::ArchivePage {
                proof: proof.clone(),
                own_author: None,
            },
        };
        self.prepare_page_exact(selection, page, action, provenance, caller, rng)
    }

    /// Fetch the exact missing parent proved by a child on this archive page.
    /// An own-outbox fallback still requires the holder's original Admission.
    pub fn prepare_archive_parent_request(
        self,
        selection: &NativeTextJoinedRowSelection,
        page: &NativeTextJoinedHistoryPage,
        dependency: &NativeTextJoinedArchiveDependency,
        holder_outbox: bool,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedDependencyExchange, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let HistoryPage::Archive { proof, .. } = &page.page else {
            return Err(Refusal::Stale);
        };
        if !Arc::ptr_eq(proof, &dependency.page) || !page.query.is_current(selection) {
            return Err(Refusal::Stale);
        }
        let provenance = DependencyProvenance::ArchivePage {
            proof: proof.clone(),
            own_author: holder_outbox.then_some(page.query.admission),
        };
        self.prepare_page_exact(selection, page, dependency.parent, provenance, caller, rng)
    }

    fn prepare_page_exact(
        self,
        selection: &NativeTextJoinedRowSelection,
        page: &NativeTextJoinedHistoryPage,
        action: ActionId,
        provenance: DependencyProvenance,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedDependencyExchange, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
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
            (None, Some(&page.query.signal), provenance),
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
            let access = source
                .node
                .member_history_access(source.prefix, query.channel)
                .map_err(|_| Refusal::Failed)?;
            match &response.request {
                HistoryRequest::Own(request) => access
                    .open_reply(query.peer, request, &response.wire)
                    .map(HistoryPage::Own)
                    .map_err(|_| Refusal::Failed),
                HistoryRequest::Archive(request, after) => {
                    let page = access
                        .open_archive_page_reply(query.peer, request, &response.wire)
                        .map_err(|_| Refusal::Failed)?;
                    if after
                        .zip(page.actions().first())
                        .is_some_and(|(previous, first)| *first <= previous)
                    {
                        return Err(Refusal::Failed);
                    }
                    let highest = page.actions().last().copied().or(*after);
                    Ok(HistoryPage::Archive {
                        page,
                        highest,
                        proof: Arc::new(ArchivePageProof(())),
                    })
                }
            }
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
    fn read_selected_own_history_action(
        &self,
        selection: &NativeTextJoinedRowSelection,
        action: ActionId,
    ) -> Option<MemberSavedOriginal> {
        let mount = self.store.installed_member_outbox_mount().ok()??;
        let key = member_outbox_key(self.store).ok()?;
        // An unavailable point lookup leaves the ordinary exact-fetch path
        // open. It supplies no evidence of either coverage or absence.
        mount
            .read_selected_history_action(
                self.store,
                selection.original.member_outbox,
                &key,
                action,
            )
            .ok()?
    }

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
