//! Actual completed warm pass -> separately saved small terminal closure. Parsed terminal
//! DATA never constructs these private-field receipts. The old SavedFiles API stays strict
//! about its own arm being current; this NEW phase governs exact terminal retries instead.
use super::*;

/// An event witness installed before changing the selected RAM journal. No cell backlink,
/// key, session, sender or task lives here; retained closure/output handles cannot form a cycle.
pub(in crate::store) struct OriginalSourceTerminal {
    operation: Arc<()>,
    arm: Arc<()>,
    run: Arc<FounderSourceRun>,
    output: Arc<Mutex<FounderSourceOutput>>,
    target: PathBuf,
}
pub(in crate::store) struct PreparedBootstrapSourceTerminal {
    cell: Arc<Mutex<FounderSourceDebt>>,
    original: Arc<OriginalSourceTerminal>,
}
pub(in crate::store) struct SavedBootstrapSourceTerminal {
    cell: Arc<Mutex<FounderSourceDebt>>,
    original: Arc<OriginalSourceTerminal>,
}
impl std::fmt::Debug for PreparedBootstrapSourceTerminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedBootstrapSourceTerminal")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for SavedBootstrapSourceTerminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedBootstrapSourceTerminal")
            .finish_non_exhaustive()
    }
}
impl OriginalSourceTerminal {
    /// Pure retained-original identity/transition check. No disk bytes/decoded count can
    /// substitute for the actual completed run and actual saved arm retained as the parent.
    pub(in crate::store::persistence) fn validate_journal(
        &self,
        journal: &FounderSourcePreparedJournal,
    ) -> Result<(), AppError> {
        use critical_namespaces::refused;
        if !journal
            .terminal
            .as_ref()
            .is_some_and(|original| std::ptr::eq(original.as_ref(), self))
            || !Arc::ptr_eq(&journal.operation, &self.operation)
            || journal.execution.is_some()
            || self.run.state.load(Ordering::Acquire) != DRAINED
        {
            return Err(refused());
        }
        let arm = journal.parent.as_deref().ok_or_else(refused)?;
        require_saved_source_journal(arm)?;
        if !Arc::ptr_eq(&arm.operation, &self.arm)
            || !Arc::ptr_eq(&self.arm, &self.run.operation)
            || !arm
                .execution
                .as_ref()
                .is_some_and(|run| Arc::ptr_eq(run, &self.run))
            || arm.terminal.is_some()
            || journal.data.io_spent != arm.data.io_spent
            || journal.data.sequence != arm.data.sequence.checked_add(1).ok_or_else(refused)?
            || journal.data.parent
                != Some(founder::IntentId::of(
                    arm.candidate.as_ref().ok_or_else(refused)?,
                ))
        {
            return Err(refused());
        }
        match (&arm.data.kind, &journal.data.kind) {
            (
                founder::IntentKind::SourcePrepared {
                    context,
                    source,
                    marker,
                },
                founder::IntentKind::SourceSaved {
                    context: c,
                    source: s,
                    marker: m,
                },
            ) if context == c && source == s && marker == m => Ok(()),
            _ => Err(refused()),
        }
    }
    /// LAST bounded output proof, after all source/current/Root waits. Never wait for a live
    /// file-I/O mutex under Root and never repeat a full source read using the old spent arm.
    pub(in crate::store::persistence) fn validate_output_last(&self) -> Result<(), AppError> {
        use critical_namespaces::refused;
        let output = self.output.try_lock().map_err(|_| refused())?;
        let completed = output.completed.as_ref().ok_or_else(refused)?;
        if !Arc::ptr_eq(&completed.run, &self.run) {
            return Err(refused());
        }
        validate_output_names(&self.target, &output, completed.lengths)
    }
}

impl ServerStore {
    /// Only an actual completed writer may open the new terminal phase. Every fallible
    /// proof/preallocation precedes the final RAM ownership move; no caller path/frame/bool
    /// selects this transition. Old SavedFiles is deliberately not called after that move.
    pub(in crate::store) fn prepare_original_source_terminal<T, R>(
        &self,
        captured: &CapturedFounderInvocation,
        guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
        registered: &RegisteredBootstrapSource,
        saved: &SavedBootstrapSourceFiles,
    ) -> Result<PreparedBootstrapSourceTerminal, AppError>
    where
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    {
        use critical_namespaces::refused;
        saved.validate_original(self, captured, guard, registered)?;
        let root_guard = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
        let root = root_guard.as_ref().ok_or_else(refused)?;
        let registry = self.founder_sources.lock().map_err(|_| refused())?;
        if !Arc::ptr_eq(&saved.cell, &registered.cell)
            || !registry
                .slots
                .iter()
                .flatten()
                .any(|cell| Arc::ptr_eq(cell, &registered.cell))
        {
            return Err(refused());
        }
        let mut actual = registered.cell.lock().map_err(|_| refused())?;
        validate_registered_source_under_root(self, captured, guard, &actual, root)?;
        let arm = actual.prepared.as_ref().ok_or_else(refused)?;
        require_saved_source_journal(arm)?;
        if !Arc::ptr_eq(&arm.operation, &saved.run.operation)
            || !arm
                .execution
                .as_ref()
                .is_some_and(|run| Arc::ptr_eq(run, &saved.run))
            || arm.terminal.is_some()
            || saved.run.state.load(Ordering::Acquire) != DRAINED
        {
            return Err(refused());
        }
        let founder::IntentKind::SourcePrepared {
            context,
            source,
            marker,
        } = &arm.data.kind
        else {
            return Err(refused());
        };
        let operation = Arc::new(());
        let original = Arc::new(OriginalSourceTerminal {
            operation: operation.clone(),
            arm: arm.operation.clone(),
            run: saved.run.clone(),
            output: actual.output.clone(),
            target: target_path(self, &actual)?,
        });
        let data = founder::IntentData {
            invocation: arm.data.invocation,
            debit: arm.data.debit,
            sequence: arm.data.sequence.checked_add(1).ok_or_else(refused)?,
            parent: Some(founder::IntentId::of(
                arm.candidate.as_ref().ok_or_else(refused)?,
            )),
            io_spent: arm.data.io_spent,
            kind: founder::IntentKind::SourceSaved {
                context: context.clone(),
                source: *source,
                marker: *marker,
            },
        };
        data.successor(
            Some((&arm.data, data.parent.ok_or_else(refused)?)),
            captured.data(),
            guard.debit_id(),
        )?;
        let mut next = Box::new(FounderSourcePreparedJournal {
            candidate: None,
            seed: None,
            marker: Zeroizing::new(arm.marker.to_vec()),
            data,
            operation,
            admitted: None,
            saved: None,
            seed_file: None,
            stage_file: None,
            parent: None,
            execution: None,
            terminal: Some(original.clone()),
        });
        let prepared = PreparedBootstrapSourceTerminal {
            cell: registered.cell.clone(),
            original,
        };
        recheck_source_capture_after_waits(
            self,
            &captured.original,
            root,
            &actual.founding,
            actual.prepared.as_ref(),
        )?;
        prepared.original.validate_output_last()?;
        if captured.original.negative.is_cancelled() || root.negative.is_cancelled() {
            return Err(refused());
        }
        // The concrete closure is retained BEFORE changing the selected phase. From now on
        // every retry uses this exact witness/parent, never the arm-current SavedFiles API.
        next.parent = actual.prepared.take().map(Box::new);
        actual.prepared = Some(*next);
        drop(actual);
        drop(registry);
        drop(root_guard);
        Ok(prepared)
    }

    /// Lost-return recovery of the actual retained closure only. This cannot adopt cold
    /// SourceSaved bytes or a foreign context/run whose labels happen to match.
    pub(in crate::store) fn lookup_original_source_terminal<T, R>(
        &self,
        captured: &CapturedFounderInvocation,
        guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
        registered: &RegisteredBootstrapSource,
    ) -> Result<Option<PreparedBootstrapSourceTerminal>, AppError>
    where
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    {
        use critical_namespaces::refused;
        registered.validate_original(self, captured, guard)?;
        let original = {
            let actual = registered.cell.lock().map_err(|_| refused())?;
            let Some(original) = actual
                .prepared
                .as_ref()
                .and_then(|journal| journal.terminal.clone())
            else {
                return Ok(None);
            };
            original
        };
        let prepared = PreparedBootstrapSourceTerminal {
            cell: registered.cell.clone(),
            original,
        };
        prepared.validate_original(self, captured, guard, registered)?;
        Ok(Some(prepared))
    }

    pub(in crate::store) fn save_original_source_terminal<T, R>(
        &self,
        captured: &CapturedFounderInvocation,
        guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
        registered: &RegisteredBootstrapSource,
        prepared: &PreparedBootstrapSourceTerminal,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<SavedBootstrapSourceTerminal, AppError>
    where
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    {
        prepared.validate_original(self, captured, guard, registered)?;
        self.save_original_source_prepared(captured, guard, registered, rng)?;
        let saved = SavedBootstrapSourceTerminal {
            cell: prepared.cell.clone(),
            original: prepared.original.clone(),
        };
        saved.validate_original(self, captured, guard, registered)?;
        Ok(saved)
    }
}

impl PreparedBootstrapSourceTerminal {
    fn validate_original<T, R>(
        &self,
        store: &ServerStore,
        captured: &CapturedFounderInvocation,
        guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
        registered: &RegisteredBootstrapSource,
    ) -> Result<(), AppError>
    where
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    {
        validate_terminal_capture(
            store,
            captured,
            guard,
            registered,
            &self.cell,
            &self.original,
            TerminalObservation::Prepared,
        )
    }
}
impl SavedBootstrapSourceTerminal {
    pub(in crate::store) fn validate_original<T, R>(
        &self,
        store: &ServerStore,
        captured: &CapturedFounderInvocation,
        guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
        registered: &RegisteredBootstrapSource,
    ) -> Result<(), AppError>
    where
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    {
        validate_terminal_capture(
            store,
            captured,
            guard,
            registered,
            &self.cell,
            &self.original,
            TerminalObservation::Saved,
        )
    }
}

// Internal observation strength, selected only by the two concrete private-field receipts.
// No request/caller flag can skip barriers or manufacture the saved observation.
enum TerminalObservation {
    Prepared,
    Saved,
}
fn validate_terminal_capture<T, R>(
    store: &ServerStore,
    captured: &CapturedFounderInvocation,
    guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
    registered: &RegisteredBootstrapSource,
    cell: &Arc<Mutex<FounderSourceDebt>>,
    original: &Arc<OriginalSourceTerminal>,
    observation: TerminalObservation,
) -> Result<(), AppError>
where
    T: catcoms_rt::MeshTransport,
    R: catcoms_rt::CryptoRngCore,
{
    use critical_namespaces::refused;
    if !Arc::ptr_eq(cell, &registered.cell) {
        return Err(refused());
    }
    let root_guard = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
    let root = root_guard.as_ref().ok_or_else(refused)?;
    let registry = store.founder_sources.lock().map_err(|_| refused())?;
    if !registry
        .slots
        .iter()
        .flatten()
        .any(|entry| Arc::ptr_eq(entry, cell))
    {
        return Err(refused());
    }
    let actual = cell.lock().map_err(|_| refused())?;
    let journal = actual.prepared.as_ref().ok_or_else(refused)?;
    if !journal
        .terminal
        .as_ref()
        .is_some_and(|entry| Arc::ptr_eq(entry, original))
        || !Arc::ptr_eq(&original.output, &actual.output)
        || original.target != target_path(store, &actual)?
    {
        return Err(refused());
    }
    original.validate_journal(journal)?;
    if matches!(observation, TerminalObservation::Saved) {
        require_saved_source_journal(journal)?;
    }
    // Exact actual current/pending selection, original source and Root, then the terminal's
    // LAST named-output proof and cheap negatives. Never re-read a large file under Root.
    validate_registered_source_under_root(store, captured, guard, &actual, root)
}
