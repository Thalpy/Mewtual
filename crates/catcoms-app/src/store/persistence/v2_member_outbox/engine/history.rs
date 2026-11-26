//! Bounded reads of every saved original, including the settled prefix.
//! Publication debt is a different view: empty debt never means empty history.
use super::*;
use catcoms_replication::chat::Body;

/// A small, resumable scan of one target's original saved revisions. A
/// successful append can only extend the authenticated ordinal prefix.
#[derive(Clone, Copy)]
pub(super) struct OwnRevisionScan {
    target: ActionId,
    channel: u128,
    next: u64,
    latest: Option<ActionId>,
}

impl MemberOutbox {
    pub(in crate::store) fn require_own_revision(
        &mut self,
        key: &[u8; 32],
        channel: u128,
        target: ActionId,
        expected: ActionId,
        payment: CatalogReadReservation,
        is_current: impl Fn() -> bool,
    ) -> Result<bool, MemberOutboxError> {
        self.read_own_revision(key, channel, target, &payment, &is_current)
            .map(|revision| revision == Some(expected))
    }

    fn read_own_revision(
        &mut self,
        key: &[u8; 32],
        channel: u128,
        target: ActionId,
        payment: &CatalogReadReservation,
        is_current: &impl Fn() -> bool,
    ) -> Result<Option<ActionId>, MemberOutboxError> {
        require_payment(payment, &self.pool, MEMBER_OUTBOX_LOOKUP_FEE)?;
        if !is_current() {
            return Err(MemberOutboxRefusal::Stale.into());
        }
        let head = self.clean()?;
        super::super::transaction::require_selected(&self.ns, head)?;
        let mut scan = match self.revision {
            Some(scan) if scan.target == target && scan.channel == channel => scan,
            _ => {
                let Some((receipt, _, original)) = self.read_action_original(key, target)? else {
                    return Ok(None);
                };
                let spec = original.action().spec();
                if spec.realm.channel() != channel || !matches!(spec.body, Body::Create { .. }) {
                    return Ok(None);
                }
                OwnRevisionScan {
                    channel,
                    target,
                    next: receipt.ordinal.checked_add(1).ok_or_else(refused)?,
                    latest: Some(target),
                }
            }
        };
        let end = head.data.next_ordinal;
        if scan.next > end {
            return Err(MemberOutboxRefusal::Stale.into());
        }
        for ordinal in scan.next..end.min(scan.next.saturating_add(MEMBER_HISTORY_MAX_ITEMS as u64))
        {
            if !is_current() {
                return Err(MemberOutboxRefusal::Stale.into());
            }
            let Some(latest) = scan.latest else { break };
            let (entry_id, entry) = self.debt_entry(key, head, ordinal)?;
            let (token, _) = self
                .token_row(key, head, entry.token)?
                .ok_or_else(refused)?;
            if token.entry != entry_id {
                return Err(refused().into());
            }
            let original = read_original(&self.ns, key, &self.context, &entry)?;
            let action = original.action();
            let spec = action.spec();
            if spec.realm.channel() == channel && spec.body.create() == Some(target) {
                scan.latest = match &spec.body {
                    Body::Edit { .. } if spec.parents.as_slice() == [latest] => Some(action.id()),
                    _ => None,
                };
            }
            scan.next = ordinal + 1;
        }
        super::super::transaction::require_selected(&self.ns, head)?;
        if !is_current() {
            return Err(MemberOutboxRefusal::Stale.into());
        }
        self.revision = Some(scan);
        match scan.latest {
            None => Ok(None),
            Some(_) if scan.next != end => Err(MemberOutboxRefusal::Busy.into()),
            latest => Ok(latest),
        }
    }

    pub(in crate::store) fn own_revision_page(
        &mut self,
        key: &[u8; 32],
        channel: u128,
        target: ActionId,
        payment: CatalogReadReservation,
        is_current: impl Fn() -> bool,
    ) -> Result<MemberHistoryPage, MemberOutboxError> {
        let revision = self.read_own_revision(key, channel, target, &payment, &is_current)?;
        let head = self.clean()?;
        let mut items = Vec::new();
        if let Some(revision) = revision {
            let (receipt, status, original) = self
                .read_action_original(key, revision)?
                .ok_or_else(refused)?;
            let spec = original.action().spec();
            let matches_target = match spec.body {
                Body::Create { .. } => revision == target,
                Body::Edit { create, .. } => create == target,
                _ => false,
            };
            if spec.realm.channel() != channel || !matches_target {
                return Err(refused().into());
            }
            items.push(MemberHistoryItem {
                receipt,
                status,
                original,
            });
        }
        super::super::transaction::require_selected(&self.ns, head)?;
        if !is_current() {
            return Err(MemberOutboxRefusal::Stale.into());
        }
        Ok(MemberHistoryPage {
            context: self.context.id,
            selected_head: head.id,
            next: head.data.next_ordinal,
            end_exclusive: head.data.next_ordinal,
            items,
            _payment: payment,
        })
    }
}

/// Independent history-page limit. Changing publication checkpoint batching
/// must not silently increase retained plaintext or the read invoice here.
pub(in crate::store) const MEMBER_HISTORY_MAX_ITEMS: usize = 8;
/// Eight maximum originals (encoded and typed), one lookup's bounded scratch,
/// and receipt/status/vector controls. Logical inventory, not an RSS estimate.
pub(in crate::store) const MEMBER_OUTBOX_HISTORY_FEE: usize = 5 * 1024 * 1024;
const _: () = assert!(
    MEMBER_HISTORY_MAX_ITEMS * 2 * catcoms_replication::chat::MAX_ACTION_BYTES
        + MEMBER_OUTBOX_LOOKUP_FEE
        + 16 * 1024
        <= MEMBER_OUTBOX_HISTORY_FEE
);

/// Bounded range DATA passed between the owning runtime and its original Host.
#[derive(Clone, Copy)]
pub(in crate::store) enum MemberHistoryRead {
    Originals {
        start: u64,
        end: Option<u64>,
        maximum: usize,
    },
    OwnRevision {
        channel: u128,
        target: ActionId,
    },
}

/// One immutable saved original and its currently recorded local disposition.
/// Neither Pending nor Submitted proves peer receipt or remote durability.
pub(in crate::store) struct MemberHistoryItem {
    receipt: MemberReceiptData,
    status: MemberEntryStatus,
    original: AssembledActionOriginal,
}
impl MemberHistoryItem {
    pub(in crate::store) fn receipt(&self) -> &MemberReceiptData {
        &self.receipt
    }
    pub(in crate::store) fn status(&self) -> MemberEntryStatus {
        self.status
    }
    pub(in crate::store) fn action(&self) -> &SignedAction {
        self.original.action()
    }
    pub(in crate::store) fn original_bytes(&self) -> &[u8] {
        self.original.original_bytes()
    }
}
impl std::fmt::Debug for MemberHistoryItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberHistoryItem")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// A finite prefix read. Reuse end_exclusive on subsequent pages so incoming
/// appends cannot extend the scan indefinitely. Ordinals are context-local DATA,
/// not capabilities. Dispositions reflect each page's own selected head, rather
/// than a status snapshot spanning pages. The payment outlives all plaintext.
pub(in crate::store) struct MemberHistoryPage {
    context: MemberOutboxContextId,
    selected_head: MemberHeadId,
    next: u64,
    end_exclusive: u64,
    items: Vec<MemberHistoryItem>,
    _payment: CatalogReadReservation,
}
impl MemberHistoryPage {
    pub(in crate::store) fn context(&self) -> MemberOutboxContextId {
        self.context
    }
    pub(in crate::store) fn selected_head(&self) -> MemberHeadId {
        self.selected_head
    }
    pub(in crate::store) fn next(&self) -> u64 {
        self.next
    }
    pub(in crate::store) fn end_exclusive(&self) -> u64 {
        self.end_exclusive
    }
    pub(in crate::store) fn items(&self) -> &[MemberHistoryItem] {
        &self.items
    }
}
impl std::fmt::Debug for MemberHistoryPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberHistoryPage")
            .field("items", &self.items.len())
            .finish_non_exhaustive()
    }
}

impl MemberOutbox {
    /// Read a bounded page of saved originals, without recovery, mutation,
    /// publication or membership admission. An uncertain owner refuses until
    /// its existing recovery path settles it. Missing/corrupt files are errors,
    /// never an empty page. start==end is a legitimate terminal empty page.
    pub(in crate::store) fn history_page(
        &self,
        key: &[u8; 32],
        start: u64,
        end_exclusive: Option<u64>,
        maximum: usize,
        payment: CatalogReadReservation,
    ) -> Result<MemberHistoryPage, MemberOutboxError> {
        require_payment(&payment, &self.pool, MEMBER_OUTBOX_HISTORY_FEE)?;
        let head = self.clean()?;
        let end = end_exclusive.unwrap_or(head.data.next_ordinal);
        if maximum == 0
            || maximum > MEMBER_HISTORY_MAX_ITEMS
            || start > end
            || end > head.data.next_ordinal
        {
            return Err(refused().into());
        }
        let count = (end - start).min(maximum as u64);
        let next = start.checked_add(count).ok_or_else(refused)?;
        let mut items = Vec::with_capacity(count as usize);
        for ordinal in start..next {
            let (entry_id, entry) = self.debt_entry(key, head, ordinal)?;
            // Both ordinal and token indices must name this exact immutable
            // entry. Do not manufacture a receipt from a partially valid pair.
            let (row, _) = self
                .token_row(key, head, entry.token)?
                .ok_or_else(refused)?;
            if row.entry != entry_id {
                return Err(refused().into());
            }
            let receipt = receipt_of(&self.context, entry.token, &row, &entry);
            let status = self.status_of(key, head, ordinal)?;
            let original = read_original(&self.ns, key, &self.context, &entry)?;
            items.push(MemberHistoryItem {
                receipt,
                status,
                original,
            });
        }
        Ok(MemberHistoryPage {
            context: self.context.id,
            selected_head: head.id,
            next,
            end_exclusive: end,
            items,
            _payment: payment,
        })
    }
}
