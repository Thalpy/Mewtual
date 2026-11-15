//! A distinct UI-authorized entry for reopening the SAME warm saved source.
//! This is not cold restore or permission to repeat fresh founder creation.
//! Root must additionally own the exact completed handoff, saved terminal, paid
//! read and old-SDK-drained evidence. None can be reconstructed from these
//! lifecycle counters or from decoded Pending/SourceSaved labels.
use super::*;

/// One warm SDK reopen, separately accounted from fresh `FounderSdk` entry.
/// Prepare outside writers; keep this and its Ends outside those writers on
/// success, refusal and unwind. It contains negative metadata, not SDK custody.
pub struct NativeChatWarmSdkReopenCut {
    ui: Arc<UiSession>,
    source: Arc<Source>,
    entered: bool,
}

impl NativeChatSourceOwner {
    /// Only the concrete retained Root producer may pair this with its real
    /// warm handoff evidence. A current UI command alone proves no saved source.
    pub fn prepare_warm_sdk_reopen(
        &self,
        command: &NativeChatUiCommand,
    ) -> Result<NativeChatWarmSdkReopenCut, NativeChatKeyRefusal> {
        let ui = command
            .original
            .upgrade()
            .ok_or(NativeChatKeyRefusal::Unavailable)?;
        if !Arc::ptr_eq(&ui.host, &self.original.host) {
            return Err(NativeChatKeyRefusal::Foreign);
        }
        if !ui.current() || !self.original.counter.open() {
            return Err(NativeChatKeyRefusal::Unavailable);
        }
        Ok(NativeChatWarmSdkReopenCut {
            ui,
            source: self.original.clone(),
            entered: false,
        })
    }
}

impl NativeChatWarmSdkReopenCut {
    /// Original-source/current-generation negatives only. This neither spends
    /// an SDK entry nor validates the physical saved-source handoff.
    pub fn preflight(
        &self,
        original: &NativeChatSourceObservation,
    ) -> Result<(), NativeChatKeyRefusal> {
        if !std::ptr::eq(Arc::as_ptr(&self.source), original.original.as_ptr()) {
            return Err(NativeChatKeyRefusal::Foreign);
        }
        if !self.ui.current() || !self.source.counter.open() {
            return Err(NativeChatKeyRefusal::Unavailable);
        }
        Ok(())
    }

    /// At the actual SDK loader, AFTER framing/semantic/payment/handoff checks.
    /// No allocation, metadata upgrade, lock or callback occurs in production.
    /// A partial entry stays in caller-owned Ends; it must never be refunded or
    /// destroyed under an enclosing writer after a later refusal.
    pub fn enter(
        &mut self,
        original: &NativeChatSourceObservation,
        ends: &mut NativeChatSdkRestoreEnds,
    ) -> Result<(), NativeChatKeyRefusal> {
        if self.entered || ends.end.is_some() {
            return Err(NativeChatKeyRefusal::AlreadyEntered);
        }
        #[cfg(test)]
        observe_cut_boundary(CutBoundary::BeforePrecheck);
        self.preflight(original)?;
        #[cfg(test)]
        observe_cut_boundary(CutBoundary::BeforeUiCounter);
        self.ui.counter.enter()?;
        self.entered = true;
        let end = ends.end.insert(SdkRestoreEnd {
            authority: self.ui.counter.clone(),
            source: self.source.clone(),
            source_entered: false,
        });
        #[cfg(test)]
        observe_cut_boundary(CutBoundary::AfterUiCounter);
        end.source.counter.enter()?;
        end.source_entered = true;
        #[cfg(test)]
        observe_cut_boundary(CutBoundary::BeforeFinal);
        self.preflight(original)
    }
}

struct SdkRestoreEnd {
    authority: Arc<Counter>,
    source: Arc<Source>,
    source_entered: bool,
}
impl Drop for SdkRestoreEnd {
    fn drop(&mut self) {
        if self.source_entered {
            self.source.counter.leave();
        }
        self.authority.leave();
    }
}

/// One original SDK restore/reopen's partial or complete counter custody.
/// Retain OUTSIDE all nested Root/native guards through caching the actual
/// result. Drained counters do not prove SDK/worker destruction or key erasure.
pub struct NativeChatSdkRestoreEnds {
    end: Option<SdkRestoreEnd>,
}
impl NativeChatSdkRestoreEnds {
    pub fn new() -> Self {
        Self { end: None }
    }
}
impl Default for NativeChatSdkRestoreEnds {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(generation: Arc<AtomicU64>) -> NativeChatUiBinding {
        NativeChatUiBinding::from_trusted_host(
            generation,
            Arc::new(AtomicU64::new(7)),
            Arc::new(AtomicBool::new(false)),
            Arc::new(()),
        )
    }

    #[test]
    fn warm_reopen_has_distinct_entry_and_original_source_identity() {
        let binding = binding(Arc::new(AtomicU64::new(7)));
        let ui = binding.open_session(7, Arc::new(())).unwrap();
        let source =
            NativeChatSourceOwner::for_trusted_source(&ui.observe(), Arc::new(())).unwrap();
        let foreign =
            NativeChatSourceOwner::for_trusted_source(&ui.observe(), Arc::new(())).unwrap();
        let mut fresh = source
            .prepare_cut(&ui.observe(), NativeChatKeyPurpose::FounderSdk)
            .unwrap();
        let mut fresh_ends = NativeChatKeyEnds::new();
        fresh
            .enter(
                &source.observe(),
                NativeChatKeyPurpose::FounderSdk,
                &mut fresh_ends,
            )
            .unwrap();
        let mut warm = source.prepare_warm_sdk_reopen(&ui.observe()).unwrap();
        let mut ends = NativeChatSdkRestoreEnds::new();
        assert_eq!(
            warm.enter(&foreign.observe(), &mut ends),
            Err(NativeChatKeyRefusal::Foreign)
        );
        warm.enter(&source.observe(), &mut ends).unwrap();
        assert_eq!(
            warm.enter(&source.observe(), &mut ends),
            Err(NativeChatKeyRefusal::AlreadyEntered)
        );
        let drain = ui.retire();
        drop(fresh_ends);
        assert!(
            !drain.is_drained(),
            "warm SDK custody remains independently entered"
        );
        drop(ends);
        assert!(drain.is_drained());
    }

    #[test]
    fn warm_reopen_rejects_an_equal_generation_from_a_foreign_host() {
        let generation = Arc::new(AtomicU64::new(7));
        let original = binding(generation.clone());
        let foreign = binding(generation);
        let ui = original.open_session(7, Arc::new(())).unwrap();
        let other = foreign.open_session(7, Arc::new(())).unwrap();
        let source =
            NativeChatSourceOwner::for_trusted_source(&ui.observe(), Arc::new(())).unwrap();
        assert!(matches!(
            source.prepare_warm_sdk_reopen(&other.observe()),
            Err(NativeChatKeyRefusal::Foreign)
        ));
    }

    #[test]
    fn warm_reopen_observes_generation_changes_at_every_final_boundary() {
        for boundary in [
            CutBoundary::BeforePrecheck,
            CutBoundary::BeforeUiCounter,
            CutBoundary::BeforeCounterCas,
            CutBoundary::AfterUiCounter,
            CutBoundary::BeforeFinal,
        ] {
            let generation = Arc::new(AtomicU64::new(7));
            let binding = binding(generation.clone());
            let ui = binding.open_session(7, Arc::new(())).unwrap();
            let source =
                NativeChatSourceOwner::for_trusted_source(&ui.observe(), Arc::new(())).unwrap();
            let mut cut = source.prepare_warm_sdk_reopen(&ui.observe()).unwrap();
            CUT_BOUNDARY.with(|slot| {
                *slot.borrow_mut() = Some((
                    boundary,
                    Box::new(move || {
                        generation.store(8, Ordering::SeqCst);
                    }),
                ))
            });
            let mut ends = NativeChatSdkRestoreEnds::new();
            assert_eq!(
                cut.enter(&source.observe(), &mut ends),
                Err(NativeChatKeyRefusal::Unavailable)
            );
            assert!(!cut.ui.current());
            let partial = ends.end.is_some();
            let drain = ui.retire();
            assert_eq!(drain.is_drained(), !partial);
            drop(ends);
            assert!(drain.is_drained());
        }
    }

    #[test]
    fn warm_reopen_retains_partial_and_full_entries_when_source_close_wins() {
        for boundary in [CutBoundary::AfterUiCounter, CutBoundary::BeforeFinal] {
            let binding = binding(Arc::new(AtomicU64::new(7)));
            let ui = binding.open_session(7, Arc::new(())).unwrap();
            let source =
                NativeChatSourceOwner::for_trusted_source(&ui.observe(), Arc::new(())).unwrap();
            let mut cut = source.prepare_warm_sdk_reopen(&ui.observe()).unwrap();
            let negative = source.original.counter.clone();
            CUT_BOUNDARY.with(|slot| {
                *slot.borrow_mut() =
                    Some((boundary, Box::new(move || negative.close_admission_sc())))
            });
            let mut ends = NativeChatSdkRestoreEnds::new();
            assert_eq!(
                cut.enter(&source.observe(), &mut ends),
                Err(NativeChatKeyRefusal::Unavailable)
            );
            let both_entered = boundary == CutBoundary::BeforeFinal;
            assert_eq!(ends.end.as_ref().unwrap().source_entered, both_entered);
            let drain = ui.retire();
            assert!(!drain.is_drained());
            assert_eq!(source.observe_retirement().is_drained(), !both_entered);
            drop(ends);
            assert!(drain.is_drained());
            assert!(source.observe_retirement().is_drained());
        }
    }
}
