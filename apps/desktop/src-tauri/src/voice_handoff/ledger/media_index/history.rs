//! Native-only reclamation of exact terminal negotiation history. The independent media
//! entry remains the paid no-revival tombstone; no physical Session is declared collected.
use super::*;

impl Entry {
    /// Match EXACTLY the owner equivalence used by Key::matches. A second physical Session
    /// or equivalent selected snapshot must not recreate an old purpose after row removal.
    pub(in crate::voice_handoff::ledger) fn tombstones(
        &self,
        selected: &SelectedHandoffOwner,
        purpose: Purpose,
    ) -> bool {
        purpose == Purpose::Derive(self.purpose)
            && self
                .selected
                .grant_identity()
                .ptr_eq(&selected.grant_identity())
            && self.selected.executor() == selected.executor()
            && self.selected.lease().is_same(selected.lease())
    }
    fn owns_row(&self, row: &Row) -> bool {
        row.key.matches(&self.selected, row.key.purpose)
            && (row.id == self.binding.parent
                || row
                    .key
                    .parent
                    .as_ref()
                    .is_some_and(|parent| parent.id == self.binding.parent))
    }
    /// Called only under Index, in the same Index -> Row order as normal collection. This
    /// authorizes a live-lease exception only for the latched exact terminal subtree.
    pub(in crate::voice_handoff::ledger) fn retiring_row(&self, row: &Row) -> bool {
        self.owns_row(row)
            && matches!(
                self.state.lock().unwrap_or_else(|e| e.into_inner()).history,
                History::Retiring(_)
            )
    }
}
impl Target<'_> {
    #[cfg(test)]
    pub(in crate::voice_handoff) fn native_history_collected_for_test(&self) -> bool {
        matches!(
            self.0.state.lock().unwrap().history,
            History::Collected { .. }
        )
    }
    /// Recover one irreversible terminal-forget operation. The caller already owns the
    /// authenticated actual Session/claim lookup, but deliberately owns NO ResolvedRow.
    /// Pending is Capacity, never success inferred from missing or merely drained rows.
    pub(in crate::voice_handoff) fn retire_history(&self) -> Result<Receipt, VoiceError> {
        self.1.request_media_history_retirement(&self.0)?;
        // Existing paid native maintenance performs collection. Do not put its32-row
        // scratch/close graph into this physical Control envelope. A later exact request
        // observes completion only after that maintenance's actual destructors returned.
        let state = super::super::transition::try_lock(&self.0.state)?;
        if !state.history_collected() {
            return Err(VoiceError::Capacity);
        }
        Ok(Receipt {
            proof: self.0.proof,
            admitted: true,
            closed: state.closed,
            released: state.released,
            current: false,
            retire_in: None,
        })
    }
}
impl Ledger {
    /// Retain a REAL accepted incoming confirmation Action independently of its native
    /// row. It has already Returned, but its original fixed Root funding still exists.
    /// Used only to distinguish native scratch drainage from actual Root owner return.
    #[cfg(test)]
    pub(in crate::voice_handoff) fn hold_root_history_for_test(
        &self,
        selected: &SelectedHandoffOwner,
        parent: RowId,
    ) -> Arc<CryptoAction> {
        let index = self.index.lock().unwrap();
        index
            .rows
            .iter()
            .flatten()
            .find_map(|row| {
                if !row.key.matches(selected, row.key.purpose)
                    || !row
                        .key
                        .parent
                        .as_ref()
                        .is_some_and(|original| original.id == parent)
                {
                    return None;
                }
                let state = row.state.lock().unwrap();
                match &state.body {
                    Body::Drained(_, _, Some(action)) => Some(action.clone()),
                    _ => None,
                }
            })
            .expect("actual accepted incoming confirmation history")
    }
    /// A forgotten first peer must not remain discovery's first candidate forever.
    /// This predicate deliberately differs from lookup_original: compact terminal history
    /// suppresses production but cannot masquerade as an extant heavyweight Row/history.
    pub(in crate::voice_handoff) fn derivation_known(
        &self,
        selected: &SelectedHandoffOwner,
        selector: PairSelector,
    ) -> bool {
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let purpose = Purpose::Derive(selector);
        index
            .rows
            .iter()
            .flatten()
            .any(|row| row.key.matches(selected, purpose))
            || index
                .media
                .iter()
                .flatten()
                .any(|entry| entry.tombstones(selected, purpose))
    }
    fn request_media_history_retirement(&self, entry: &Arc<Entry>) -> Result<(), VoiceError> {
        let index = super::super::transition::try_lock(&self.index)?;
        if !index
            .media
            .iter()
            .flatten()
            .any(|actual| Arc::ptr_eq(actual, entry))
        {
            return Err(VoiceError::Unauthorized);
        }
        if !matches!(
            super::super::transition::try_lock(&entry.state)?.history,
            History::Live
        ) {
            return Ok(()); // Exact lost reply, including after all old rows were destroyed.
        }
        let parent = index
            .rows
            .iter()
            .flatten()
            .find(|row| row.id == entry.binding.parent)
            .ok_or(VoiceError::Unauthorized)?;
        if !parent
            .key
            .matches(&entry.selected, Purpose::Derive(entry.purpose))
        {
            return Err(VoiceError::Unauthorized);
        }
        let state = super::super::transition::try_lock(&parent.state)?;
        let delivery = state.final_delivery().ok_or(VoiceError::Capacity)?;
        if !matches!(state.body, Body::Drained(..))
            || state.status.action
                != Some((
                    entry.binding.action.serial,
                    entry.binding.action.version,
                    entry.binding.action.input_tag,
                ))
            || delivery.kind != FinalKind::Plan
            || !delivery.acknowledged
            || !Arc::ptr_eq(&delivery.session, &entry.session)
            || !Self::final_retirement_ready(&state)
        {
            return Err(VoiceError::Capacity);
        }
        let proof = delivery.issued.ok_or(VoiceError::Capacity)?;
        let root_history = state.plan().ok_or(VoiceError::Capacity)?.action_history()?;
        // All checks precede the irreversible latch. No Clock/provider/allocation or
        // affine destruction runs under Index/Row/entry-state guards.
        let mut target = super::super::transition::try_lock(&entry.state)?;
        target.root_history = Some(root_history);
        target.history = History::Retiring(proof);
        Ok(())
    }
    /// Called only AFTER collection scratch owners have been destroyed outside guards.
    /// The prior native latch distinguishes actual completed removal from unknown history.
    pub(in crate::voice_handoff::ledger) fn finish_media_history_collection(
        &self,
        collector: &super::super::collection::Collector<'_>,
    ) {
        if !std::ptr::eq(self, collector.ledger()) {
            return;
        }
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        for entry in index.media.iter().flatten() {
            let mut state = entry.state.lock().unwrap_or_else(|e| e.into_inner());
            if let History::Retiring(proof) = state.history {
                if !index.rows.iter().flatten().any(|row| entry.owns_row(row)) {
                    // Only this affine collector can certify actual native destruction.
                    // Atomics only: no Actor/RPC wait, allocation, Clock or affine Drop
                    // beneath Index/Entry. Existing Root maintenance performs its GC.
                    let Some(history) = state.root_history.as_ref() else {
                        continue; // Missing original capability cannot manufacture completion.
                    };
                    history.request_collection();
                    state.history = History::Collected {
                        _final_proof: proof,
                    };
                }
            }
        }
    }
}
