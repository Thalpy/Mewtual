//! C-side one-hop file reads. Native read intent, MLS catalog observation, helper route and
//! actual transport custody are separate owners. Local preparation never spends coverage or
//! durable Keep allowance; the actor commits the exact prepared choice after its durable debit.
use super::{routes::RoutePlan, wire::*};
use crate::*;
use catcoms_mls::group::{MemberCatalogCursor, MemberCatalogEntry, MemberCatalogView};
use catcoms_rt::{ConnectionLease, RequestAuthority, SharedRequestKeepalive};
use std::sync::Weak;
use std::time::Duration;
use tokio::sync::watch;

mod search;

const HELPERS: usize = 4;
const REQUEST_MS: u64 = 12_000;
const REQUEST_INTERVAL_MS: u64 = 150;
const REQUEST_PACING_ROWS: usize = 128;

/// Local identity of an exact encrypted reference, including its wrapped key. Bound before
/// encoding: malicious metadata cannot turn a 64KiB page preparation into an unbounded copy.
pub fn bridge_reference_variant(reference: &FileRef) -> Result<[u8; 32], BridgeError> {
    if reference.size > catcoms_storage::CHUNK_PAD_CEILING as u64
        || reference.mime.len() > 4096
        || reference.wrapped_key.ciphertext.len() != 48
    {
        return Err(BridgeError::Bounds);
    }
    Ok(*blake3::hash(&reference.encode()).as_bytes())
}

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("file bridge capacity is busy")]
    Busy,
    #[error("file bridge read is no longer authorized")]
    Stale,
    #[error("file bridge request exceeds its bounds")]
    Bounds,
}

#[cfg(test)]
mod tests;

/// Opaque local selection, not a renderer-supplied source list or a reusable wire grant.
#[derive(Clone, Debug)]
pub struct BridgeChoice(search::Choice);
impl BridgeChoice {
    pub fn helper_peer(&self) -> PeerId {
        self.0.helper.peer
    }
    pub fn variant(&self) -> [u8; 32] {
        self.0.variant
    }
}
#[derive(Debug)]
pub enum BridgeStep {
    Ready(BridgeChoice),
    /// One bounded helper/source catalog page was examined; let another actor turn run.
    Yield,
    /// A bounded retry sweep is due after fresh source coverage; apply ordinary read backoff.
    Backoff,
    /// This read's observed choices are exhausted, not a claim of global file absence.
    Exhausted,
}

#[derive(Debug)]
struct Origin {
    caller: RequestCancellation,
    native: RequestCancellation,
    actor: Arc<dyn RequestAuthority>,
    parent: Arc<dyn RequestAuthority>,
    root: Arc<dyn RequestAuthority>,
}
#[async_trait::async_trait]
impl RequestAuthority for Origin {
    fn authorized(&self) -> bool {
        !self.caller.is_cancelled()
            && !self.native.is_cancelled()
            && self.actor.authorized()
            && self.parent.authorized()
            && self.root.authorized()
    }
    async fn revoked(&self) {
        let mut caller = self.caller.clone();
        let mut native = self.native.clone();
        if self.authorized() {
            tokio::select! {
                _ = caller.cancelled() => {},
                _ = native.cancelled() => {},
                _ = self.actor.revoked() => {},
                _ = self.parent.revoked() => {},
                _ = self.root.revoked() => {},
            }
        }
    }
}

/// One chunk's original native read scope and continuing source coverage. Helpers have at most
/// four epoch-bound indexed key attestations; bootstrap examines at most 64 raw leaves per turn.
/// Repeated bootstrap passes admit a newly connected D without resetting any source cursor.
pub struct BridgeCatalog {
    instance: Arc<()>,
    origin: Arc<Origin>,
    view: MemberCatalogView,
    bootstrap: MemberCatalogCursor,
    helpers: Vec<MemberCatalogEntry>,
    search: search::Search,
    bootstrap_turn: bool,
    choice: Option<BridgeChoice>,
    committed: bool,
}
impl fmt::Debug for BridgeCatalog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FileBridgeCatalog { .. }")
    }
}

#[derive(Debug)]
pub(crate) struct RequesterScope {
    live: watch::Receiver<bool>,
    origin: Arc<Origin>,
    connection: ConnectionLease,
    clock: Arc<dyn Clock + Send>,
    deadline: u64,
    _route: SharedRequestKeepalive,
}
impl RequesterScope {
    pub(crate) fn deadline(&self) -> u64 {
        self.deadline
    }
}
#[async_trait::async_trait]
impl RequestAuthority for RequesterScope {
    fn authorized(&self) -> bool {
        *self.live.borrow()
            && self.live.has_changed().is_ok()
            && self.origin.authorized()
            && self.connection.is_live()
            && self.clock.monotonic_ms() < self.deadline
    }
    async fn revoked(&self) {
        let mut live = self.live.clone();
        while self.authorized() {
            tokio::select! {
                _ = live.changed() => {},
                _ = self.origin.revoked() => return,
                _ = self.connection.revoked() => return,
                _ = self.clock.sleep(Duration::from_millis(self.deadline.saturating_sub(self.clock.monotonic_ms()))) => {},
            }
        }
    }
}
pub(super) struct Registration {
    scope: Weak<RequesterScope>,
    live: watch::Sender<bool>,
    helper: MemberCatalogEntry,
    descriptor: DescriptorRef,
}

/// Response authentication retains the original scope through AEAD and storage admission.
pub(crate) struct BridgeContext {
    instance: Arc<()>,
    view: MemberCatalogView,
    grant: Vec<u8>,
    helper: MemberCatalogEntry,
    sources: Vec<MemberCatalogEntry>,
    pub(crate) scope: Arc<RequesterScope>,
}
pub struct PreparedBridgePage {
    pub(crate) peer: PeerId,
    pub(crate) frame: Vec<u8>,
    pub(crate) context: BridgeContext,
    choice: BridgeChoice,
}
impl fmt::Debug for PreparedBridgePage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PreparedFileBridgePage { .. }")
    }
}
impl PreparedBridgePage {
    /// This original deadline is fixed before durable work debit; later polling cannot renew it.
    pub fn deadline(&self) -> u64 {
        self.context.scope.deadline
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Native-only entry point. Ordinary trusted Rust reads lacking a captured native origin
    /// remain direct-only. No observer below grants a manifest, destination or new Keep budget.
    pub fn begin_file_bridge_catalog(
        &mut self,
        caller: RequestCancellation,
        parent: Arc<dyn RequestAuthority>,
    ) -> Result<BridgeCatalog, BridgeError> {
        if !self.policy_allows_service() || caller.is_cancelled() || !parent.authorized() {
            return Err(BridgeError::Stale);
        }
        let native = self
            .file_service
            .native
            .as_ref()
            .ok_or(BridgeError::Stale)?
            .clone();
        let origin = Arc::new(Origin {
            caller,
            native: native.session,
            actor: native.actor,
            parent,
            root: self
                .file_effect_authority()
                .map_err(|_| BridgeError::Stale)?,
        });
        if !origin.authorized() {
            return Err(BridgeError::Stale);
        }
        // Never lazily rebuild the O(roster) native observation from an explicit-read page.
        let view = self
            .group
            .current_member_catalog()
            .ok_or(BridgeError::Stale)?;
        Ok(BridgeCatalog {
            instance: self.blob_fetch_instance.clone(),
            origin,
            bootstrap: view.start(),
            helpers: Vec::new(),
            search: search::Search::new(self.device.device_id(), view.clone()),
            view,
            bootstrap_turn: true,
            choice: None,
            committed: false,
        })
    }

    fn bridge_catalog_current(&self, catalog: &BridgeCatalog) -> bool {
        Arc::ptr_eq(&catalog.instance, &self.blob_fetch_instance)
            && catalog.origin.authorized()
            && catalog.view.validate(&self.group).is_ok()
            && self.group.is_active()
    }

    /// Metadata-only route lookup: a catalog entry attests current MLS identity. Do not call
    /// contains_device/member_signature_key/voice_bound_routes here: each walks the whole tree.
    fn bridge_helper_descriptor(&self, helper: &MemberCatalogEntry) -> Option<DescriptorRef> {
        if self.member_peers.len() > MAX_PEER_RECORDS
            || self.peer_records.len() > MAX_PEER_RECORDS
            || helper.validate(&self.group).is_err()
        {
            return None;
        }
        let device = helper.device_id();
        let descriptor = self.peer_records.get(&device)?;
        let peer = PeerId::new(descriptor.peer_id);
        if !self
            .member_peers
            .iter()
            .any(|p| p.bound && p.device == device && p.peer == peer)
            || self
                .peer_records
                .iter()
                .any(|(other, record)| *other != device && record.peer_id == descriptor.peer_id)
            || !self
                .transport
                .connection_lease(peer)
                .is_some_and(|lease| lease.is_live())
        {
            return None;
        }
        Some(DescriptorRef {
            device,
            peer,
            seq: descriptor.seq,
            hash: peer_descriptor_hash(descriptor),
        })
    }

    pub(crate) fn reconcile_file_requesters(&mut self) {
        self.file_service
            .requesters
            .retain(|row| row.scope.strong_count() != 0);
        for row in &self.file_service.requesters {
            if self.bridge_helper_descriptor(&row.helper).as_ref() != Some(&row.descriptor) {
                row.live.send_replace(false);
            }
        }
    }

    fn bridge_send_ready(&mut self, helper: DeviceId, now: u64) -> bool {
        self.file_service
            .requester_ready
            .retain(|(_, until)| *until > now);
        self.file_service.requester_ready.len() < REQUEST_PACING_ROWS
            && !self
                .file_service
                .requester_ready
                .iter()
                .any(|(device, _)| *device == helper)
    }

    /// One call examines either one helper bootstrap page OR one source page. The helper cache
    /// never evicts/restarts existing helper/variant coverage when another member reconnects.
    pub fn next_file_bridge_choice(
        &mut self,
        catalog: &mut BridgeCatalog,
        variants: &[[u8; 32]],
    ) -> Result<BridgeStep, BridgeError> {
        if !self.bridge_catalog_current(catalog) || variants.len() > 4 {
            return Err(BridgeError::Stale);
        }
        if let Some(old) = catalog
            .choice
            .as_ref()
            .filter(|p| !variants.contains(&p.variant()))
        {
            catalog.search.retire_pending(&old.0);
            catalog.choice = None;
            catalog.committed = false;
        }
        // Requesting another selection relinquishes the last admitted batch. An old clone
        // cannot start more work while bootstrap yields or the remaining catalog is empty.
        // Keep an unstarted proposal intact: local capacity refusal never spends coverage.
        if catalog.committed {
            catalog.choice = None;
            catalog.committed = false;
        }
        if variants.is_empty() {
            catalog.choice = None;
            return Ok(BridgeStep::Exhausted);
        }
        if catalog.bootstrap_turn && catalog.helpers.len() < HELPERS {
            let page = catalog
                .view
                .page(
                    &self.group,
                    &catalog.bootstrap,
                    self.device.device_id(),
                    self.device.device_id(),
                )
                .map_err(|_| BridgeError::Stale)?;
            for entry in page.entries {
                if catalog.helpers.len() < HELPERS
                    && !catalog
                        .helpers
                        .iter()
                        .any(|h| h.device_id() == entry.device_id())
                    && self.bridge_helper_descriptor(&entry).is_some()
                {
                    catalog.helpers.push(entry);
                }
            }
            catalog.bootstrap = if page.complete {
                catalog.view.start()
            } else {
                page.next
            };
            catalog.bootstrap_turn = false;
            return Ok(BridgeStep::Yield);
        }
        catalog.bootstrap_turn = true;
        let helpers: Vec<_> = catalog
            .helpers
            .iter()
            .filter_map(|entry| {
                self.bridge_helper_descriptor(entry)
                    .map(|d| search::Helper {
                        device: d.device,
                        peer: d.peer,
                    })
            })
            .collect();
        let step = catalog
            .search
            .propose(&self.group, variants, &helpers)
            .map_err(|_| BridgeError::Stale)?;
        Ok(match step {
            search::Step::Ready(choice) => {
                let choice = BridgeChoice(choice);
                if !catalog
                    .choice
                    .as_ref()
                    .is_some_and(|old| old.0.same_as(&choice.0))
                {
                    catalog.committed = false;
                }
                catalog.choice = Some(choice.clone());
                BridgeStep::Ready(choice)
            }
            search::Step::Yield => BridgeStep::Yield,
            search::Step::Backoff => BridgeStep::Backoff,
            // Further bootstrap can still discover a newly connected D. Its original read
            // deadline/finite work allowance bounds this search, not a false global absence.
            search::Step::Exhausted if catalog.helpers.len() < HELPERS => BridgeStep::Yield,
            search::Step::Exhausted => BridgeStep::Exhausted,
        })
    }

    /// Rebind an explicitly retried batch to the helper's current authenticated PeerId. Source
    /// coverage stays unchanged; an ABA route change creates a new local ticket, so old copies
    /// cannot become current again. Existing requests retain their original connection lease.
    pub fn refresh_file_bridge_choice(
        &self,
        catalog: &mut BridgeCatalog,
        choice: &BridgeChoice,
    ) -> Result<BridgeChoice, BridgeError> {
        if !self.bridge_catalog_current(catalog)
            || !catalog
                .choice
                .as_ref()
                .is_some_and(|p| p.0.same_as(&choice.0))
        {
            return Err(BridgeError::Stale);
        }
        let entry = catalog
            .helpers
            .iter()
            .find(|entry| entry.device_id() == choice.0.helper.device)
            .ok_or(BridgeError::Stale)?;
        let descriptor = self
            .bridge_helper_descriptor(entry)
            .ok_or(BridgeError::Stale)?;
        let refreshed = catalog
            .search
            .refresh(
                &choice.0,
                search::Helper {
                    device: descriptor.device,
                    peer: descriptor.peer,
                },
            )
            .map_err(|_| BridgeError::Stale)?;
        let refreshed = BridgeChoice(refreshed);
        catalog.choice = Some(refreshed.clone());
        Ok(refreshed)
    }

    /// Prepare after application FileRef authorization, before irreversible durable spend.
    /// Root reserves its route record here. Busy leaves the exact pending source batch intact.
    pub fn prepare_file_bridge_page(
        &mut self,
        catalog: &BridgeCatalog,
        choice: &BridgeChoice,
        reference: &FileRef,
        offset: u32,
        length: usize,
    ) -> Result<PreparedBridgePage, BridgeError> {
        self.reconcile_file_requesters();
        if !self.bridge_catalog_current(catalog)
            || !catalog
                .choice
                .as_ref()
                .is_some_and(|p| p.0.same_as(&choice.0))
        {
            return Err(BridgeError::Stale);
        }
        if bridge_reference_variant(reference)? != choice.variant() {
            return Err(BridgeError::Bounds);
        }
        if reference.size > catcoms_storage::CHUNK_PAD_CEILING as u64
            || length == 0
            || length > MAX_PAGE_BYTES
        {
            return Err(BridgeError::Bounds);
        }
        let bound = catcoms_storage::padded_len(
            reference.size as usize,
            catcoms_storage::CHUNK_PAD_FLOOR,
            catcoms_storage::CHUNK_PAD_CEILING,
        ) + catcoms_storage::PAD_FOOTER_BYTES
            + 40;
        if offset as usize >= bound {
            return Err(BridgeError::Bounds);
        }
        let helper = catalog
            .helpers
            .iter()
            .find(|h| h.device_id() == choice.0.helper.device)
            .ok_or(BridgeError::Stale)?
            .clone();
        let descriptor = self
            .bridge_helper_descriptor(&helper)
            .ok_or(BridgeError::Stale)?;
        if descriptor.peer != choice.helper_peer() {
            return Err(BridgeError::Stale);
        }
        if !self.bridge_send_ready(descriptor.device, self.clock.monotonic_ms()) {
            return Err(BridgeError::Busy);
        }
        for source in &choice.0.sources {
            source
                .validate(&self.group)
                .map_err(|_| BridgeError::Stale)?;
        }
        let connection = self
            .transport
            .connection_lease(descriptor.peer)
            .filter(ConnectionLease::is_live)
            .ok_or(BridgeError::Stale)?;
        let mut routes: RoutePlan = self.file_service.routes.source().ok_or(BridgeError::Busy)?;
        let deadline = self
            .clock
            .monotonic_ms()
            .checked_add(REQUEST_MS)
            .ok_or(BridgeError::Stale)?;
        let (live, signal) = watch::channel(true);
        let scope = Arc::new(RequesterScope {
            live: signal,
            origin: catalog.origin.clone(),
            connection,
            clock: self.clock.clone(),
            deadline,
            _route: routes.take_scope().ok_or(BridgeError::Busy)?,
        });
        if !scope.authorized() {
            return Err(BridgeError::Stale);
        }
        let mut nonce = [0; 16];
        self.rng.fill_bytes(&mut nonce);
        let sources: Vec<_> = choice
            .0
            .sources
            .iter()
            .map(MemberCatalogEntry::device_id)
            .collect();
        let frame = GrantFields {
            group: &self.group.group_id(),
            epoch: self.group.epoch(),
            helper: descriptor.device,
            helper_peer: descriptor.peer,
            requester_peer: self.local_peer(),
            window: Window {
                cid: reference.ciphertext_cid,
                max_cipher_bytes: bound as u32,
                offset,
                length: length.min(bound - offset as usize) as u32,
            },
            sources: &sources,
            // Use the same rollback-resistant timestamp domain as B/A admission. The local
            // twelve-second scope still uses this Root's independent monotonic clock.
            created_ms: self
                .file_service
                .admission
                .request_time_ms()
                .map_err(|_| BridgeError::Stale)?,
            nonce,
        }
        .issue(&self.device)
        .map_err(|_| BridgeError::Bounds)?;
        self.file_service.requesters.push(Registration {
            scope: Arc::downgrade(&scope),
            live,
            helper: helper.clone(),
            descriptor: descriptor.clone(),
        });
        Ok(PreparedBridgePage {
            peer: descriptor.peer,
            frame: frame.clone(),
            choice: choice.clone(),
            context: BridgeContext {
                instance: catalog.instance.clone(),
                view: catalog.view.clone(),
                grant: frame,
                helper,
                sources: choice.0.sources.clone(),
                scope,
            },
        })
    }

    /// The actor calls this only after a successful durable Keep debit. Refusal after that debit
    /// is conservative spent work, never a general attempt refund after an ambiguous handoff.
    pub fn commit_file_bridge_page(
        &mut self,
        catalog: &mut BridgeCatalog,
        prepared: PreparedBridgePage,
    ) -> Result<PendingBlobFetch<T>, BridgeError> {
        self.reconcile_file_requesters();
        if !self.bridge_catalog_current(catalog)
            || !prepared.context.scope.authorized()
            || !catalog
                .choice
                .as_ref()
                .is_some_and(|p| p.0.same_as(&prepared.choice.0))
        {
            return Err(BridgeError::Stale);
        }
        let now = self.clock.monotonic_ms();
        let next = now
            .checked_add(REQUEST_INTERVAL_MS)
            .ok_or(BridgeError::Stale)?;
        if !self.bridge_send_ready(prepared.choice.0.helper.device, now) {
            return Err(BridgeError::Busy);
        }
        if !self.bridge_catalog_current(catalog) || !prepared.context.scope.authorized() {
            return Err(BridgeError::Stale);
        }
        if !catalog.committed {
            if !catalog.search.admitted(&prepared.choice.0) {
                return Err(BridgeError::Stale);
            }
            catalog.committed = true;
        }
        self.file_service
            .requester_ready
            .push((prepared.choice.0.helper.device, next));
        Ok(PendingBlobFetch::bridge(self.transport.clone(), prepared))
    }

    pub fn file_bridge_unsupported(&self, catalog: &mut BridgeCatalog, choice: &BridgeChoice) {
        if self.bridge_catalog_current(catalog)
            && catalog
                .choice
                .as_ref()
                .is_some_and(|p| p.0.same_as(&choice.0))
        {
            catalog.search.unsupported(choice.0.helper.device);
            catalog.choice = None;
            catalog.committed = false;
        }
    }

    /// An authenticated Unavailable or exhausted ambiguous retry is an observation of temporary
    /// failure. Remember a bounded later sweep; it neither proves absence nor refills allowance.
    pub fn file_bridge_unavailable(&self, catalog: &mut BridgeCatalog, choice: &BridgeChoice) {
        if self.bridge_catalog_current(catalog)
            && catalog
                .choice
                .as_ref()
                .is_some_and(|p| p.0.same_as(&choice.0))
        {
            catalog.search.retry_batch(&choice.0);
        }
    }

    pub(crate) fn authenticate_bridge_page(
        &self,
        context: BridgeContext,
        response: Result<Bytes, TransportError>,
    ) -> Result<BlobPageOutcome, SyncError> {
        if !Arc::ptr_eq(&context.instance, &self.blob_fetch_instance)
            || context.view.validate(&self.group).is_err()
            || !context.scope.authorized()
            || context.helper.validate(&self.group).is_err()
        {
            return Err(SyncError::Unauthorized);
        }
        let response = response?;
        if response.is_empty() {
            return Ok(BlobPageOutcome::Unsupported);
        }
        let grant = Grant::parse(&context.grant).map_err(|_| SyncError::Malformed)?;
        let reply = HelperPage::parse(&response, &grant).map_err(|_| SyncError::Malformed)?;
        if reply.public_key() != context.helper.signature_key() || !reply.verify_signature() {
            return Err(SyncError::Unauthorized);
        }
        match reply.status() {
            HelperStatus::Absent => Ok(BlobPageOutcome::Absent),
            HelperStatus::Unavailable => Ok(BlobPageOutcome::Unavailable),
            HelperStatus::Found => {
                let page = reply.source().ok_or(SyncError::Malformed)?;
                let entry = context
                    .sources
                    .iter()
                    .find(|entry| entry.device_id() == page.source())
                    .ok_or(SyncError::Unauthorized)?;
                if entry.validate(&self.group).is_err()
                    || page.public_key() != entry.signature_key()
                    || !page.verify_signature()
                    || !context.scope.authorized()
                {
                    return Err(SyncError::Unauthorized);
                }
                let offset = grant.window().offset;
                Ok(BlobPageOutcome::Page(Box::new(BlobPage {
                    bytes: page.page().to_vec(),
                    offset,
                    total: page.total(),
                    more: offset as usize + page.page().len() < page.total() as usize,
                    provider: roles::fingerprint(&page.source()),
                })))
            }
        }
    }
}
