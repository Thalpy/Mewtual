//! Bounded reads of every saved original, including the settled prefix.
//! Publication debt is a different view: empty debt never means empty history.
use super::*;

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
pub(in crate::store) struct MemberHistoryRead {
    pub(in crate::store) start: u64,
    pub(in crate::store) end: Option<u64>,
    pub(in crate::store) maximum: usize,
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
