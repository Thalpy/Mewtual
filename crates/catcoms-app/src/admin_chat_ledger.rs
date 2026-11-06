//! Privileged retained-original reads, separate from the all-member moderation projection.
//!
//! A read observes a finite signed-log prefix, never today's winning message list. It can
//! therefore finish under continuous chat traffic and still expose overwritten/deleted text.
//! Graph-only history and typed owner evidence are not supplied by this first consumer: every
//! reply explicitly reports incomplete coverage. This module grants no authoring/admission right.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use automerge::transaction::Transactable;

use catcoms_replication::doc::{SignedHistoryError, SignedHistoryIndex, SignedHistoryRow};
use catcoms_replication::history_inspection::LegacyChangeInspection;
use catcoms_rt::{CryptoRngCore, MeshTransport, RequestCancellation};
use catcoms_sync::{read_published_roster_bounded, ROLES_DOC};
use catcoms_wire::DocType;

use crate::Server;

/// Logical setup-work threshold on the insert-only document table's declared capacity.
/// This is not an exact physical bucket/allocator bound or a channel/history retention limit:
/// refusal never removes documents or requests user repair. See bounded_documents' invariant.
const MAX_DOCUMENT_BUCKETS: usize = 4096;
const MAX_READER_MEMBERS: usize = 256;
const MAX_READER_ADMINS: usize = 64;
const VIEW_TTL_MS: u64 = 120_000;
// Conservative explicit allocation invoice: fixed map/source/Hasher state; <=4096 channel
// descriptors including their decimal String allocations; and <=1024B for each fixed row,
// key, BTree node/occupancy overhead. No original payload is copied into the cache. Allocator
// bookkeeping/slack and already-retained document storage are outside this logical invoice.
const CACHE_BASE_BYTES: usize = 32 * 1024;
// The index contains an inline resumable BLAKE3 Hasher even while its Option is None.
// Pay the actual descriptor layout; another128B covers its bounded row-prefix allocation.
const CHANNEL_BYTES: usize = std::mem::size_of::<ChannelIndex>() + 128;
const ORIGINAL_ROW_BYTES: usize = 1024;

/// Aggregate process/host allowance shared by all its privileged read scopes. A native host
/// should create one budget, not one per request/server. Exhaustion affects only this view.
#[derive(Debug)]
pub struct AdminReadBudget {
    limit: usize,
    used: AtomicUsize,
}
impl AdminReadBudget {
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            used: AtomicUsize::new(0),
        })
    }
    pub fn charged_bytes(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }
    fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Reservation, AdminLedgerError> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.limit)
            })
            .map_err(|_| AdminLedgerError::Capacity)?;
        Ok(Reservation {
            owner: self.clone(),
            bytes,
        })
    }
}
#[derive(Debug)]
struct Reservation {
    owner: Arc<AdminReadBudget>,
    bytes: usize,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.owner.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// Native-only observation scope. The cancellation must observe the ORIGINAL unlock and
/// registry-row lifetime; cloning a handle cannot keep either publisher alive. Reuse one scope
/// for that exact session/row, and create a fresh one after unlock/replacement. This token is
/// never accepted from IPC, serialized, or sufficient for role authorization on its own.
#[derive(Debug, Clone)]
pub struct AdminReadScope {
    identity: Arc<()>,
    cancellation: RequestCancellation,
    budget: Arc<AdminReadBudget>,
}
impl AdminReadScope {
    pub fn new(cancellation: RequestCancellation, budget: Arc<AdminReadBudget>) -> Self {
        Self {
            identity: Arc::new(()),
            cancellation,
            budget,
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }
}

/// Read outcomes are deliberately independent of persistence/network/authoring errors. Do not
/// turn a busy, unsupported or stale projection into a failed message or a history repair task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminLedgerError {
    SessionUnavailable,
    NotAuthorized,
    InvalidRequest,
    Busy,
    Capacity,
    EvidenceUnavailable,
    Stale,
}
impl From<SignedHistoryError> for AdminLedgerError {
    fn from(error: SignedHistoryError) -> Self {
        match error {
            SignedHistoryError::Stale => Self::Stale,
            SignedHistoryError::Capacity => Self::Capacity,
            SignedHistoryError::Frame => Self::EvidenceUnavailable,
            SignedHistoryError::Busy => Self::Busy,
            SignedHistoryError::Request => Self::InvalidRequest,
        }
    }
}

/// Exact original metadata. Namespace is deliberately separate from the legacy signed scope;
/// v1 attribution proves neither historical group membership nor historical moderation rights.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminLegacyRow {
    pub row_id: String,
    pub channel: u128,
    pub namespace_group: Vec<u8>,
    pub original: SignedHistoryRow,
}
#[derive(Debug)]
pub struct AdminLedgerPage {
    pub revision: String,
    pub rows: Vec<AdminLegacyRow>,
    pub known_row_count: usize,
    pub next_cursor: Option<String>,
    /// Always true in this vertical: graph-only and typed evidence remain independent work.
    pub incomplete: bool,
}
#[derive(Debug)]
pub struct AdminLedgerDetail {
    pub revision: String,
    pub row: AdminLegacyRow,
    pub frame_offset: usize,
    pub frame_bytes: Vec<u8>,
    pub next_offset: Option<usize>,
    pub signature: [u8; 64],
    pub decoded: LegacyChangeInspection,
}

struct ChannelIndex {
    id: u128,
    row_prefix: String,
    originals: SignedHistoryIndex,
}
/// Ephemeral actor-owned cache. No cursor map, original body copy, or authorizing sender is
/// retained. Exactly one captured view per Server; a different native scope drops the old one.
#[derive(Default)]
pub(crate) struct AdminLedgerCache {
    view: Option<View>,
}
struct View {
    scope: AdminReadScope,
    revision: String,
    epoch: u64,
    admin_generation: u64,
    roles: Option<RoleSource>,
    channels: Vec<ChannelIndex>,
    next_channel: usize,
    ready: bool,
    idle_until: u64,
    _reservation: Reservation,
}
struct RoleSource {
    originals: SignedHistoryIndex,
    graph: automerge::HistoryInstanceId,
    applied: usize,
    queued: usize,
}
impl RoleSource {
    fn capture(doc: &catcoms_replication::EncryptedDoc) -> Result<Self, AdminLedgerError> {
        if doc.doc().pending_ops() != 0 {
            return Err(AdminLedgerError::Busy);
        }
        Ok(Self {
            originals: doc.signed_history_index(doc.op_count())?,
            graph: doc.doc().history_instance_id(),
            applied: doc.doc().applied_change_hashes().len(),
            queued: doc.doc().queued_changes().len(),
        })
    }
    fn matches(&self, doc: &catcoms_replication::EncryptedDoc) -> bool {
        self.originals.is_current_version(doc)
            && self.graph == doc.doc().history_instance_id()
            && self.applied == doc.doc().applied_change_hashes().len()
            && self.queued == doc.doc().queued_changes().len()
            && doc.doc().pending_ops() == 0
    }
}
impl std::fmt::Debug for AdminLedgerCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminLedgerCache")
            .field("has_view", &self.view.is_some())
            .finish()
    }
}

fn canonical_hex(value: &str, bytes: usize) -> bool {
    value.len() == bytes * 2
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn channel_id(value: &str) -> Result<u128, AdminLedgerError> {
    if value.len() > 39 {
        return Err(AdminLedgerError::InvalidRequest);
    }
    let id: u128 = value
        .parse()
        .map_err(|_| AdminLedgerError::InvalidRequest)?;
    if id.to_string() != value {
        return Err(AdminLedgerError::InvalidRequest);
    }
    Ok(id)
}
fn parse_row(value: &str) -> Result<(u128, [u8; 32]), AdminLedgerError> {
    if value.len() > 111 {
        return Err(AdminLedgerError::InvalidRequest);
    }
    let mut fields = value.split(':');
    if fields.next() != Some("legacy") {
        return Err(AdminLedgerError::InvalidRequest);
    }
    let id = channel_id(fields.next().ok_or(AdminLedgerError::InvalidRequest)?)?;
    let hash = fields.next().ok_or(AdminLedgerError::InvalidRequest)?;
    if fields.next().is_some() || !canonical_hex(hash, 32) {
        return Err(AdminLedgerError::InvalidRequest);
    }
    let hash = hex::decode(hash).map_err(|_| AdminLedgerError::InvalidRequest)?;
    Ok((
        id,
        hash.try_into()
            .map_err(|_| AdminLedgerError::InvalidRequest)?,
    ))
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Run before cursor/revision validation and before any privileged payload access. Bound
    /// the published roster read separately; the older `my_role` display path clones its scalar.
    /// This inherits the documented R7 freshness limit of a locally known owner-signed roster;
    /// it does not turn that shared copy into MLS Add or typed action admission authority.
    fn require_admin_ledger_reader(&self, scope: &AdminReadScope) -> Result<(), AdminLedgerError> {
        if scope.is_cancelled() {
            return Err(AdminLedgerError::SessionUnavailable);
        }
        if self.sync.member_count() > MAX_READER_MEMBERS
            || !self.sync.contains_member(&self.device_id)
        {
            return Err(AdminLedgerError::NotAuthorized);
        }
        let owner = self
            .sync
            .designated_committer_id()
            .ok_or(AdminLedgerError::NotAuthorized)?;
        if self.sync.group_id_ref().is_empty() || self.sync.group_id_ref().len() > 256 {
            return Err(AdminLedgerError::EvidenceUnavailable);
        }
        if owner == self.device_id {
            return Ok(());
        }
        let allowed = self
            .sync
            .doc(DocType::MemberRoles, ROLES_DOC)
            .filter(|doc| doc.doc().pending_ops() == 0)
            .and_then(|doc| {
                read_published_roster_bounded(
                    doc.doc(),
                    self.sync.group_id_ref(),
                    &owner,
                    MAX_READER_ADMINS,
                )
            })
            .is_some_and(|admins| admins.contains(&self.device_id));
        if !allowed {
            return Err(AdminLedgerError::NotAuthorized);
        }
        Ok(())
    }

    fn start_admin_ledger(&mut self, scope: &AdminReadScope) -> Result<View, AdminLedgerError> {
        let inventory = self
            .sync
            .bounded_documents(MAX_DOCUMENT_BUCKETS)
            .ok_or(AdminLedgerError::Capacity)?;
        let mut channels = 0usize;
        let mut originals = 0usize;
        for ((kind, _), doc) in inventory {
            if kind == DocType::Channel {
                channels = channels.checked_add(1).ok_or(AdminLedgerError::Capacity)?;
                originals = originals
                    .checked_add(doc.op_count())
                    .ok_or(AdminLedgerError::Capacity)?;
            }
        }
        if originals > u32::MAX as usize {
            return Err(AdminLedgerError::Capacity);
        }
        let fee = originals
            .checked_mul(ORIGINAL_ROW_BYTES)
            .and_then(|fee| {
                channels
                    .checked_mul(CHANNEL_BYTES)
                    .and_then(|part| fee.checked_add(part))
            })
            .and_then(|fee| fee.checked_add(CACHE_BASE_BYTES))
            .ok_or(AdminLedgerError::Capacity)?;
        // Reserve BEFORE the channel vector, decimal keys, tree rows or Hasher are allocated.
        let reservation = scope.budget.reserve(fee)?;
        let mut captured = Vec::with_capacity(channels);
        for ((kind, id), doc) in self
            .sync
            .bounded_documents(MAX_DOCUMENT_BUCKETS)
            .ok_or(AdminLedgerError::Capacity)?
        {
            if kind == DocType::Channel {
                captured.push(ChannelIndex {
                    id,
                    row_prefix: format!("legacy:{id}:"),
                    originals: doc.signed_history_index(doc.op_count())?,
                });
            }
        }
        // Include the separator: "legacy:10:" sorts BEFORE "legacy:1:". Bare decimal IDs
        // sort oppositely for prefix cases. Unique sources need no stable sort; doing it in
        // place also avoids a stable sort's heap scratch for these large inline indices.
        captured.sort_unstable_by(|a, b| a.row_prefix.cmp(&b.row_prefix));
        let roles = self
            .sync
            .doc(DocType::MemberRoles, ROLES_DOC)
            .map(RoleSource::capture)
            .transpose()?;
        Ok(View {
            scope: scope.clone(),
            revision: self.sync.random_id(),
            epoch: self.sync.epoch(),
            admin_generation: self.sync.admin_generation(),
            roles,
            channels: captured,
            next_channel: 0,
            ready: false,
            idle_until: self
                .sync
                .runtime_clock()
                .monotonic_ms()
                .saturating_add(VIEW_TTL_MS),
            _reservation: reservation,
        })
    }

    fn view_policy_is_current(&self, view: &View, scope: &AdminReadScope) -> bool {
        view.scope.same(scope)
            && !view.scope.is_cancelled()
            && view.epoch == self.sync.epoch()
            && view.admin_generation == self.sync.admin_generation()
            && match (&view.roles, self.sync.doc(DocType::MemberRoles, ROLES_DOC)) {
                (None, None) => true,
                (Some(source), Some(doc)) => source.matches(doc),
                _ => false,
            }
            && self.sync.runtime_clock().monotonic_ms() < view.idle_until
            && view.channels.iter().all(|channel| {
                self.sync
                    .doc(DocType::Channel, channel.id)
                    .is_some_and(|doc| channel.originals.is_same_source(doc))
            })
    }

    /// The actual current captured read revision for a stale-response reset. Native may ask
    /// after a stale page/detail; it must retain the ORIGINAL Actor/scope across this await.
    /// Recheck reader rights first. Creating a missing view captures only bounded descriptors,
    /// without hashing/decoding originals, closing transactions or inventing a reset revision.
    pub fn admin_chat_ledger_revision(
        &mut self,
        scope: &AdminReadScope,
    ) -> Result<String, AdminLedgerError> {
        if let Err(error) = self.require_admin_ledger_reader(scope) {
            self.admin_chat_ledger.view = None;
            return Err(error);
        }
        if self
            .admin_chat_ledger
            .view
            .as_ref()
            .is_some_and(|view| !self.view_policy_is_current(view, scope))
        {
            self.admin_chat_ledger.view = None;
        }
        if self.admin_chat_ledger.view.is_none() {
            self.admin_chat_ledger.view = Some(self.start_admin_ledger(scope)?);
        }
        Ok(self
            .admin_chat_ledger
            .view
            .as_ref()
            .expect("captured above")
            .revision
            .clone())
    }

    /// One bounded observer turn, then a page only after this captured prefix is coherent.
    /// Null starts/refreshes a view; retrying null while busy continues that same finite build.
    /// Native must bind scope to original session/Actor custody and fence plaintext replies too.
    pub fn admin_chat_ledger_page(
        &mut self,
        scope: &AdminReadScope,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<AdminLedgerPage, AdminLedgerError> {
        if let Err(error) = self.require_admin_ledger_reader(scope) {
            self.admin_chat_ledger.view = None;
            return Err(error);
        }
        if !(1..=32).contains(&limit) || cursor.is_some_and(|cursor| cursor.len() > 512) {
            return Err(AdminLedgerError::InvalidRequest);
        }
        let parsed = cursor
            .map(|cursor| {
                let (revision, row) = cursor
                    .split_once('.')
                    .ok_or(AdminLedgerError::InvalidRequest)?;
                if !canonical_hex(revision, 16) {
                    return Err(AdminLedgerError::InvalidRequest);
                }
                Ok((revision, parse_row(row)?))
            })
            .transpose()?;
        let mut view = self.admin_chat_ledger.view.take();
        if view
            .as_ref()
            .is_some_and(|view| !self.view_policy_is_current(view, scope))
        {
            view = None;
            if parsed.is_some() {
                return Err(AdminLedgerError::Stale);
            }
        }
        if parsed.is_none() && view.as_ref().is_some_and(|view| view.ready) {
            view = None; // explicit refresh, releasing the old fee before reserving another
        }
        let mut view = match view {
            Some(view) => view,
            None => self.start_admin_ledger(scope)?,
        };
        view.idle_until = self
            .sync
            .runtime_clock()
            .monotonic_ms()
            .saturating_add(VIEW_TTL_MS);
        let result = self.read_admin_ledger_page(&mut view, parsed, limit);
        // A read failure cannot mutate originals. Drop a replaced source, but retain a bounded
        // in-progress view across ordinary Busy replies so steady traffic cannot starve it.
        if result
            .as_ref()
            .is_err_and(|error| *error == AdminLedgerError::Stale)
        {
            return result;
        }
        self.admin_chat_ledger.view = Some(view);
        result
    }

    fn read_admin_ledger_page(
        &self,
        view: &mut View,
        cursor: Option<(&str, (u128, [u8; 32]))>,
        limit: usize,
    ) -> Result<AdminLedgerPage, AdminLedgerError> {
        if cursor.is_some_and(|(revision, _)| revision != view.revision) {
            return Err(AdminLedgerError::Stale);
        }
        if view.next_channel < view.channels.len() {
            let channel = &mut view.channels[view.next_channel];
            let doc = self
                .sync
                .doc(DocType::Channel, channel.id)
                .ok_or(AdminLedgerError::Stale)?;
            if channel.originals.advance(doc)?.complete_signed_prefix {
                view.next_channel += 1;
            }
            if view.next_channel < view.channels.len() {
                return Err(AdminLedgerError::Busy);
            }
        }
        view.ready = true;
        let mut rows = Vec::with_capacity(limit);
        let mut after = cursor.map(|(_, position)| position);
        let mut found_after = after.is_none();
        for channel in &view.channels {
            let doc = self
                .sync
                .doc(DocType::Channel, channel.id)
                .ok_or(AdminLedgerError::Stale)?;
            let key = if let Some((after_channel, hash)) = after {
                if after_channel != channel.id {
                    continue;
                }
                if channel.originals.row(doc, &hash)?.is_none() {
                    return Err(AdminLedgerError::InvalidRequest);
                }
                found_after = true;
                after = None;
                Some(hash)
            } else {
                None
            };
            // Fetch one extra bounded row to decide if the cursor is genuinely exhausted.
            let wanted = limit + 1 - rows.len();
            let batch = channel.originals.page(doc, key, wanted.min(32))?;
            let last = batch.last().map(|row| row.op_hash);
            let full = batch.len() == 32;
            for original in batch {
                rows.push(AdminLegacyRow {
                    row_id: format!("{}{}", channel.row_prefix, hex::encode(original.op_hash)),
                    channel: channel.id,
                    namespace_group: self.group_id(),
                    original,
                });
                if rows.len() > limit {
                    break;
                }
            }
            // The index's individual page ceiling is32; a full32-row public page still
            // needs one bounded look-ahead in THIS channel, not merely the next channel.
            if wanted == 33 && full && rows.len() == limit {
                for original in channel.originals.page(doc, last, 1)? {
                    rows.push(AdminLegacyRow {
                        row_id: format!("{}{}", channel.row_prefix, hex::encode(original.op_hash)),
                        channel: channel.id,
                        namespace_group: self.group_id(),
                        original,
                    });
                }
            }
            if rows.len() > limit {
                break;
            }
        }
        if !found_after {
            return Err(AdminLedgerError::InvalidRequest);
        }
        // Count the entire captured inventory, including channels preceding the page position.
        // Use fixed metadata in this bounded <=4096-source sum rather than counting physical
        // source duplicates as unique rows.
        let known_row_count = view
            .channels
            .iter()
            .try_fold(0usize, |sum, channel| {
                sum.checked_add(channel.originals.known_row_count())
            })
            .ok_or(AdminLedgerError::Capacity)?;
        let next_cursor = if rows.len() > limit {
            rows.pop();
            rows.last()
                .map(|row| format!("{}.{}", view.revision, row.row_id))
        } else {
            None
        };
        Ok(AdminLedgerPage {
            revision: view.revision.clone(),
            rows,
            known_row_count,
            next_cursor,
            incomplete: true,
        })
    }

    /// Exact original detail, including overwritten/deleted mutations, without a lookup in
    /// today's messages. Chunk bytes and their original signature remain independently readable
    /// when the bounded decoder refuses a large/compressed/malformed historical change.
    pub fn admin_chat_ledger_detail(
        &mut self,
        scope: &AdminReadScope,
        row_id: &str,
        revision: &str,
        offset: usize,
    ) -> Result<AdminLedgerDetail, AdminLedgerError> {
        if let Err(error) = self.require_admin_ledger_reader(scope) {
            self.admin_chat_ledger.view = None;
            return Err(error);
        }
        if !canonical_hex(revision, 16) {
            return Err(AdminLedgerError::InvalidRequest);
        }
        let (id, hash) = parse_row(row_id)?;
        let view = self
            .admin_chat_ledger
            .view
            .as_ref()
            .ok_or(AdminLedgerError::Stale)?;
        if !self.view_policy_is_current(view, scope) || revision != view.revision {
            self.admin_chat_ledger.view = None;
            return Err(AdminLedgerError::Stale);
        }
        if !view.ready {
            return Err(AdminLedgerError::Busy);
        }
        let channel = view
            .channels
            .iter()
            .find(|channel| channel.id == id)
            .ok_or(AdminLedgerError::InvalidRequest)?;
        let doc = self
            .sync
            .doc(DocType::Channel, id)
            .ok_or(AdminLedgerError::Stale)?;
        let original = channel
            .originals
            .row(doc, &hash)?
            .ok_or(AdminLedgerError::InvalidRequest)?;
        let frame_bytes = channel
            .originals
            .original_chunk(doc, &hash, offset, 64 * 1024)?;
        let next = offset
            .checked_add(frame_bytes.len())
            .ok_or(AdminLedgerError::InvalidRequest)?;
        let next_offset = (next < original.original_bytes).then_some(next);
        let signature = channel.originals.signature(doc, &hash)?;
        let decoded = channel.originals.inspect_original_change(doc, &hash)?;
        let result = AdminLedgerDetail {
            revision: view.revision.clone(),
            row: AdminLegacyRow {
                row_id: row_id.to_owned(),
                channel: id,
                namespace_group: self.group_id(),
                original,
            },
            frame_offset: offset,
            frame_bytes,
            next_offset,
            signature,
            decoded,
        };
        if let Some(view) = &mut self.admin_chat_ledger.view {
            view.idle_until = self
                .sync
                .runtime_clock()
                .monotonic_ms()
                .saturating_add(VIEW_TTL_MS);
        }
        Ok(result)
    }

    /// Wake the real fair actor turn when the native scope retires or an idle view expires.
    /// Retain only observer/Clock handles, never the index or an authorizing sender. Without
    /// this wake a quiet locked server could retain its allocation until the next network event.
    pub(crate) fn admin_chat_ledger_wake(
        &self,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        let scope = self
            .admin_chat_ledger
            .view
            .as_ref()
            .map(|view| (view.scope.cancellation.clone(), view.idle_until));
        let clock = self.sync.runtime_clock();
        async move {
            let Some((mut cancellation, until)) = scope else {
                return std::future::pending::<()>().await;
            };
            let delay = until.saturating_sub(clock.monotonic_ms());
            tokio::select! {
                _ = cancellation.cancelled() => {},
                _ = clock.sleep(std::time::Duration::from_millis(delay)) => {},
            }
        }
    }

    /// O(1) policy/source checks plus the fixed <=4096-source inventory fence; no body lookup,
    /// crypto, network or save. Called before every fair actor turn, including the retirement wake.
    pub(crate) fn collect_admin_chat_ledger(&mut self) {
        if self
            .admin_chat_ledger
            .view
            .as_ref()
            .is_some_and(|view| !self.view_policy_is_current(view, &view.scope))
        {
            self.admin_chat_ledger.view = None;
        }
    }
}

#[cfg(test)]
mod tests;
