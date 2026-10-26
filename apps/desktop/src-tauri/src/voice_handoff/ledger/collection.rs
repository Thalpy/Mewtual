//! One affine collector owns the whole unlink/destruction/completion interval. Index
//! absence while this ticket exists is NOT evidence that the removed owner returned.
use super::*;

pub(super) struct Collector<'a>(&'a Ledger);
impl<'a> Collector<'a> {
    pub(super) fn ledger(&self) -> &'a Ledger {
        self.0
    }
    fn acquire(ledger: &'a Ledger) -> Option<Self> {
        ledger
            .collecting
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| Self(ledger))
    }
}
impl Drop for Collector<'_> {
    fn drop(&mut self) {
        self.0.collecting.store(false, Ordering::Release);
    }
}
/// At most one actual batch exists per Ledger. The collector gate is LAST so every
/// retained Row/media destructor and completion marking precede another collector.
pub(in crate::voice_handoff) struct RemovalBatch<'a> {
    rows: [Option<Arc<Row>>; ROWS],
    media: [Option<Arc<media_index::Entry>>; ROWS],
    collector: Collector<'a>,
}
impl<'a> RemovalBatch<'a> {
    fn new(collector: Collector<'a>) -> Self {
        let mut batch = Self {
            rows: std::array::from_fn(|_| None),
            media: std::array::from_fn(|_| None),
            collector,
        };
        batch.unlink();
        batch
    }
    fn unlink(&mut self) {
        let ledger = self.collector.0;
        let mut index = ledger.index.lock().unwrap_or_else(|e| e.into_inner());
        for (slot, retired) in index.media.iter_mut().zip(&mut self.media) {
            if slot.as_ref().is_some_and(|entry| entry.reclaimable()) {
                *retired = slot.take();
            }
        }
        let Index { rows, media, .. } = &mut *index;
        for (slot, retired) in rows.iter_mut().zip(&mut self.rows) {
            let reclaim = slot.as_ref().is_some_and(|row| {
                let history = media.iter().flatten().any(|entry| entry.retiring_row(row));
                if (row.key.lease.is_live() && !history) || Arc::strong_count(row) != 1 {
                    return false;
                }
                let state = row.state.lock().unwrap_or_else(|e| e.into_inner());
                (state.session_collected || history)
                    && (matches!(state.body, Body::Drained(..) | Body::ClosedPending)
                        || sdp_rows::locally_drained(&state) == Some(true))
                    && Ledger::root_retirement_ready(&state)
            });
            if reclaim {
                *retired = slot.take();
            }
        }
    }
    fn drain(&mut self) {
        // Take one owner at a time: fixed scratch stays bounded and no destructor runs
        // under Index/Row/Coordinator/UI. Children really return their parent Arcs here.
        for row in &mut self.rows {
            drop(row.take());
        }
        for entry in &mut self.media {
            drop(entry.take());
        }
    }
    #[cfg(test)]
    pub(in crate::voice_handoff) fn next_pass(&mut self) {
        self.drain();
        self.unlink();
    }
    #[cfg(test)]
    pub(in crate::voice_handoff) fn contains(&self, id: RowId) -> bool {
        self.rows.iter().flatten().any(|row| row.id == id)
    }
}
impl Drop for RemovalBatch<'_> {
    fn drop(&mut self) {
        self.drain();
        // The gate excludes ANY other unfinished scratch owner. Native latch + absence
        // now proves actual completion, including when this caller was cancelled/unwound.
        self.collector
            .0
            .finish_media_history_collection(&self.collector);
    }
}
impl Ledger {
    /// Existing dead-owner cleanup. Live history is removable only after its exact native
    /// terminal latch. Never wait for a concurrent collector, including destructor reentry.
    pub(in crate::voice_handoff) fn collect_retired(&self) {
        let Some(collector) = Collector::acquire(self) else {
            return;
        };
        self.close_retired_sdp();
        drop(RemovalBatch::new(collector));
    }
    /// The ordinary service's already paid bounded maintenance graph. No SDK/CloseTicket
    /// graph is needed for ACKed live history; canonical Root readiness remains mandatory.
    pub(in crate::voice_handoff) fn collect_media_history(&self) {
        let Some(collector) = Collector::acquire(self) else {
            return;
        };
        drop(RemovalBatch::new(collector));
    }
    #[cfg(test)]
    pub(in crate::voice_handoff) fn test_hold_collection(&self) -> Option<RemovalBatch<'_>> {
        Collector::acquire(self).map(RemovalBatch::new)
    }
}
