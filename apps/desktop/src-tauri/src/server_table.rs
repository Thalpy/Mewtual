//! One native registry, with explicit actor-backed and native-text rows.
//! Numeric IDs and display metadata never coerce a native row into an actor.
//! Owning insertion/removal uses outer slots so refusal/destruction stays out
//! of native guard scopes. Native founding must supply a real Root selection.
use super::*;
use catcoms_app::store::{
    CriticalCatalogReadPool, NativeTextJoinedRowSelection, NativeTextRuntimeSelection,
    NativeTextVaultAuthenticationCheck, NativeUiInvocationBudget, NativeUiInvocationMetadata,
};

pub(super) const NATIVE_ROWS: usize = 2;
type FounderSelection = NativeTextRuntimeSelection<catcoms_net::MeshService, OsCryptoRng>;

#[derive(Clone)]
pub(super) enum NativeSelection {
    Founder(FounderSelection),
    Joined(NativeTextJoinedRowSelection),
}

impl NativeSelection {
    pub(super) fn founder(&self) -> Option<&FounderSelection> {
        match self {
            Self::Founder(original) => Some(original),
            Self::Joined(_) => None,
        }
    }

    pub(super) fn id_data(&self) -> u64 {
        match self {
            Self::Founder(original) => original.id_data(),
            Self::Joined(original) => original.id_data(),
        }
    }

    pub(super) fn same_original(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Founder(a), Self::Founder(b)) => a.same_original(b),
            (Self::Joined(a), Self::Joined(b)) => a.same_original(b),
            _ => false,
        }
    }

    fn close_original_cached(&self) {
        match self {
            Self::Founder(original) => original.close_original_cached(),
            Self::Joined(original) => original.close_original_cached(),
        }
    }
}

enum Row {
    Legacy(Box<ServerEntry>),
    TextV2(NativeRow),
}

/// Unpublished candidate. Its Drop cancels only its own unpublished observer;
/// it MUST NOT close the Root source, which can already back an installed row
/// during an identical founding retry. Only NativeRow owns that close duty.
pub(super) struct PreparedNativeRow {
    data: Box<NativeRowData>,
}
pub(super) struct NativeRow {
    data: Box<NativeRowData>,
}
/// Bounded frozen creation DATA; lifecycle owners/payment are built internally.
pub(super) struct NativeRowSpec<'a> {
    pub(super) selection: NativeSelection,
    pub(super) instance: u64,
    pub(super) target: Arc<NativeTextVaultAuthenticationCheck>,
    pub(super) request: [u8; 32],
    pub(super) name: &'a str,
    pub(super) display_name: &'a str,
    pub(super) is_dm: bool,
    pub(super) channel: u128,
}
pub(super) struct NativeRowData {
    pub(super) selection: NativeSelection,
    files: keep_session::EntryOwner,
    pub(super) instance: u64,
    pub(super) target: Arc<NativeTextVaultAuthenticationCheck>,
    pub(super) request: [u8; 32],
    pub(super) name: String,
    pub(super) display_name: String,
    pub(super) is_dm: bool,
    pub(super) channel: u128,
    retiring: bool,
    // LAST: actual dedicated native row payment follows all retained DATA.
    _credit: NativeUiInvocationMetadata,
}
impl PreparedNativeRow {
    pub(super) fn is_for_recovered_vault(
        &self,
        vault: &catcoms_app::store::NativeTextVault,
    ) -> bool {
        matches!(&self.data.selection, NativeSelection::Joined(original)
            if original.is_current_cached() && original.is_from_original_vault_cached(vault))
            && self.data.target.is_for_original_vault(vault)
    }

    /// Called with already paid/frozen fields outside guards. No renderer DATA
    /// can construct selection or target; native must still authenticate their
    /// exact original relationship at final installation.
    pub(super) fn new(
        spec: NativeRowSpec<'_>,
        credit: NativeUiInvocationMetadata,
    ) -> Result<Self, String> {
        if spec.request == [0; 32] {
            return Err("Invalid native server details.".into());
        }
        let (name, display_name) = bounded_native_names(spec.name, spec.display_name)?;
        let data = NativeRowData {
            selection: spec.selection,
            files: keep_session::EntryOwner::new(),
            instance: spec.instance,
            target: spec.target,
            request: spec.request,
            name,
            display_name,
            is_dm: spec.is_dm,
            channel: spec.channel,
            retiring: false,
            _credit: credit,
        };
        Ok(Self {
            data: Box::new(data),
        })
    }
}

/// Copy only bounded borrowed DATA, never a caller's unbounded String capacity.
/// This constructor work occurs after payment and outside all native guards.
fn bounded_native_names(name: &str, display: &str) -> Result<(String, String), String> {
    if name.is_empty() || name.len() > 256 || display.is_empty() || display.len() > 256 {
        return Err("Invalid native server details.".into());
    }
    Ok((name.to_owned(), display.to_owned()))
}

impl NativeRow {
    pub(super) fn data(&self) -> &NativeRowData {
        &self.data
    }
    pub(super) fn scope(&self, account: RequestCancellation) -> Option<RequestCancellation> {
        (!self.data.retiring).then(|| self.data.files.scope(account))
    }
    pub(super) fn matches(
        &self,
        selection: &NativeSelection,
        instance: u64,
        target: &NativeTextVaultAuthenticationCheck,
    ) -> bool {
        !self.data.retiring
            && self.data.instance == instance
            && self.data.selection.same_original(selection)
            && self.data.target.same_original(target)
    }
    /// Only SC closure belongs under a row guard; never notify watch waiters
    /// or destroy the row there. Actual Leave/Retired remains a separate write.
    fn fence(&mut self) {
        self.data.selection.close_original_cached();
        self.data.retiring = true;
    }
}
impl Drop for NativeRow {
    fn drop(&mut self) {
        // Outside-guard backstop for the ONE installed row owner. Passive
        // PreparedNativeRow deliberately has no equivalent source close.
        self.data.selection.close_original_cached();
    }
}

/// Removed whole rows. Dispose outside UI/table/Store/Root guards. No SDK or
/// actual physical-drain proof follows from extracting these metadata owners.
pub(super) struct RemovedRows {
    _rows: HashMap<u64, Row>,
    // LAST, including empty/legacy-only maps: the actual original table payer
    // follows its bucket/control allocation, not an unrelated row's payment.
    _metadata: Arc<TableMetadata>,
}
struct TableMetadata {
    _credit: NativeUiInvocationMetadata,
}

pub(super) struct ServerTable {
    rows: HashMap<u64, Row>,
    row_budget: NativeUiInvocationBudget,
    // LAST: pays table controls and native bucket overlap, not Legacy actors.
    metadata: Arc<TableMetadata>,
}

/// A publication slot checked before the voice-registration CAS. Holding the
/// map entry keeps the checked incarnation/vacancy unchanged until publication;
/// publication itself cannot fail or destroy the previous whole owner.
pub(super) struct LegacyPublication<'table, 'removed> {
    slot: std::collections::hash_map::Entry<'table, u64, Row>,
    removed: &'removed mut Option<Box<ServerEntry>>,
}
impl LegacyPublication<'_, '_> {
    pub(super) fn publish(self, candidate: Box<ServerEntry>) {
        match self.slot {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(Row::Legacy(candidate));
            }
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                // Preserve the existing reload cancellation boundary. Retiring
                // the old file scope precedes publication, while disposing the
                // old actor/voice/mesh owner follows release of native guards.
                let Row::Legacy(previous) = slot.get() else {
                    unreachable!("checked legacy publication slot")
                };
                previous.files.retire();
                let Row::Legacy(previous) = slot.insert(Row::Legacy(candidate)) else {
                    unreachable!("checked legacy publication slot")
                };
                *self.removed = Some(previous);
            }
        }
    }
}
impl Default for ServerTable {
    fn default() -> Self {
        // Independent of authentication/generation credits: retained rows
        // cannot consume the pool required to lock or close their own vault.
        let row_budget = NativeUiInvocationBudget::new(
            CriticalCatalogReadPool::new(NativeUiInvocationBudget::required_capacity())
                .expect("fixed native row pool"),
        )
        .expect("fixed native row policy");
        let credit = row_budget.prepare().expect("fixed native table payment");
        Self {
            rows: HashMap::with_capacity(NATIVE_ROWS),
            row_budget,
            metadata: Arc::new(TableMetadata { _credit: credit }),
        }
    }
}
impl ServerTable {
    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub(super) fn contains_key(&self, id: &u64) -> bool {
        self.rows.contains_key(id)
    }
    pub(super) fn legacy(&self, id: &u64) -> Option<&ServerEntry> {
        match self.rows.get(id) {
            Some(Row::Legacy(row)) => Some(row.as_ref()),
            _ => None,
        }
    }
    pub(super) fn legacy_mut(&mut self, id: &u64) -> Option<&mut ServerEntry> {
        match self.rows.get_mut(id) {
            Some(Row::Legacy(row)) => Some(row.as_mut()),
            _ => None,
        }
    }
    pub(super) fn legacy_iter(&self) -> impl Iterator<Item = (&u64, &ServerEntry)> {
        self.rows.iter().filter_map(|(id, row)| match row {
            Row::Legacy(row) => Some((id, row.as_ref())),
            _ => None,
        })
    }
    pub(super) fn legacy_values(&self) -> impl Iterator<Item = &ServerEntry> {
        self.legacy_iter().map(|(_, row)| row)
    }
    pub(super) fn native(&self, id: &u64) -> Option<&NativeRow> {
        match self.rows.get(id) {
            Some(Row::TextV2(row)) => Some(row),
            _ => None,
        }
    }
    pub(super) fn native_iter(&self) -> impl Iterator<Item = (&u64, &NativeRow)> {
        self.rows.iter().filter_map(|(id, row)| match row {
            Row::TextV2(row) => Some((id, row)),
            _ => None,
        })
    }
    /// Resource policy clone only. Reserve actual credits AFTER releasing the
    /// short table borrow, before allocating/copying row data or taking guards.
    pub(super) fn native_budget(&self) -> NativeUiInvocationBudget {
        self.row_budget.clone()
    }
    /// Check vacancy or the exact previous Legacy incarnation before any
    /// registration effect. A TextV2 row always refuses. The empty outer slot
    /// ensures replacing a row never drops its owner under this table guard.
    pub(super) fn prepare_legacy_publication<'table, 'removed>(
        &'table mut self,
        id: u64,
        previous_instance: Option<u64>,
        removed: &'removed mut Option<Box<ServerEntry>>,
    ) -> Result<LegacyPublication<'table, 'removed>, &'static str> {
        if removed.is_some() {
            return Err("Server installation was superseded.");
        }
        let slot = self.rows.entry(id);
        let matches = match &slot {
            std::collections::hash_map::Entry::Vacant(_) => previous_instance.is_none(),
            std::collections::hash_map::Entry::Occupied(slot) => match slot.get() {
                Row::Legacy(row) => Some(row.instance) == previous_instance,
                Row::TextV2(_) => false,
            },
        };
        if !matches {
            return Err("Server installation was superseded.");
        }
        Ok(LegacyPublication { slot, removed })
    }
    pub(super) fn install_native(
        &mut self,
        candidate: &mut Option<PreparedNativeRow>,
    ) -> Result<(), &'static str> {
        let row = candidate
            .as_ref()
            .ok_or("Missing native server candidate.")?;
        let id = row.data.selection.id_data();
        if self.rows.contains_key(&id) {
            return Err("Server installation was superseded.");
        }
        if self.native_iter().count() >= NATIVE_ROWS {
            return Err("The native server limit has been reached.");
        }
        if self.rows.len() >= self.rows.capacity() {
            return Err("The server list has no prepared capacity.");
        }
        let prepared = candidate.take().expect("checked native candidate");
        self.rows.insert(
            id,
            Row::TextV2(NativeRow {
                data: prepared.data,
            }),
        );
        Ok(())
    }
    pub(super) fn extract_stopped_legacy(
        &mut self,
        stopped: leaving::StoppedServer,
        output: &mut Option<Box<ServerEntry>>,
    ) -> Result<(), &'static str> {
        let id = stopped.server_id();
        if output.is_some()
            || self
                .legacy(&id)
                .is_none_or(|row| !stopped.matches_entry(row))
        {
            return Err("The server changed before removal.");
        }
        // Preserve Legacy's existing synchronous file-work revocation before
        // removal (leaving::remove). This can notify watch waiters; unlike the
        // native counter fence it makes no atomic-only/no-notification claim.
        self.legacy(&id).expect("checked legacy row").files.retire();
        let Some(Row::Legacy(row)) = self.rows.remove(&id) else {
            unreachable!("same locked legacy row")
        };
        *output = Some(row);
        Ok(())
    }
    /// Pure-native irreversible close only. It cannot bypass Legacy's actual
    /// save/freeze/stop proof. The caller must preserve this owning output on
    /// retry and dispose outside every guard after actual source shutdown.
    pub(super) fn extract_native_for_shutdown(
        &mut self,
        output: &mut Option<RemovedRows>,
    ) -> Result<(), &'static str> {
        if output.is_some() {
            return Err("Previous server removal is still retained.");
        }
        self.fence_native_for_shutdown()?;
        *output = Some(RemovedRows {
            _rows: std::mem::take(&mut self.rows),
            _metadata: self.metadata.clone(),
        });
        Ok(())
    }
    /// Irreversible whole-vault native close, after its real prelude. Leave the
    /// owners installed while original Sources/SDK/work drain without a table
    /// guard; a failed close retries with these SAME fenced rows still present.
    pub(super) fn fence_native_for_shutdown(&mut self) -> Result<(), &'static str> {
        if self.legacy_iter().next().is_some() {
            return Err("Legacy servers still require their shutdown barrier.");
        }
        for row in self.rows.values_mut() {
            if let Row::TextV2(row) = row {
                row.fence();
            }
        }
        Ok(())
    }
}

// Row strings are bounded, and the candidate box is allocated before guards.
// This invoices actual layout/buckets plus transient metadata overlap; it is
// not a whole-process RSS or IPC/WebView serialization bound.
const _: () = assert!(
    2 * std::mem::size_of::<NativeRowData>() + 2 * std::mem::size_of::<Row>() + 4 * 256 + 1024
        <= 4096
);

// Fixture construction is intentionally unavailable in production. Existing
// actor tests may install/replace rows directly, but cannot coerce TextV2 into
// an actor or provide a production removal/stop proof.
#[cfg(test)]
impl ServerTable {
    pub(super) fn install_legacy_fixture(
        &mut self,
        id: u64,
        candidate: ServerEntry,
    ) -> Option<ServerEntry> {
        let previous = self.legacy(&id).map(|row| row.instance);
        let mut removed = None;
        self.prepare_legacy_publication(id, previous, &mut removed)
            .expect("Legacy fixture cannot overwrite a native row")
            .publish(Box::new(candidate));
        removed.map(|row| *row)
    }
    pub(super) fn remove_legacy_fixture(&mut self, id: &u64) -> Option<ServerEntry> {
        assert!(
            self.native(id).is_none(),
            "Legacy fixture cannot remove native rows"
        );
        match self.rows.remove(id) {
            Some(Row::Legacy(row)) => {
                row.files.retire();
                Some(*row)
            }
            None => None,
            _ => unreachable!("checked Legacy fixture"),
        }
    }
    pub(super) fn clear_legacy_fixture(&mut self) {
        assert!(self.native_iter().next().is_none());
        self.rows.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_names_copy_length_without_retaining_unbounded_input_capacity() {
        let mut inflated = String::with_capacity(1024 * 1024);
        inflated.push('x');
        let (name, display) = bounded_native_names(&inflated, &inflated).unwrap();
        assert_eq!((&*name, &*display), ("x", "x"));
        assert!(name.capacity() <= 256 && display.capacity() <= 256);
        assert!(bounded_native_names("", "x").is_err());
        assert!(bounded_native_names(&"x".repeat(257), "x").is_err());
    }

    #[tokio::test]
    async fn legacy_publication_refusals_and_unpublished_slot_preserve_original() {
        let (entry, task, drain) = leaving::tests::running(7);
        let (account, signal) = watch::channel(false);
        let scope = entry.files.scope(RequestCancellation::new(signal, None));
        let mut table = ServerTable::default();
        table.install_legacy_fixture(1, entry);
        let mut removed = None;
        assert!(table
            .prepare_legacy_publication(1, None, &mut removed)
            .is_err());
        assert!(table
            .prepare_legacy_publication(1, Some(6), &mut removed)
            .is_err());
        assert!(table
            .prepare_legacy_publication(2, Some(7), &mut removed)
            .is_err());
        // A failed registration discards its reservation without publication.
        {
            let _unpublished = table
                .prepare_legacy_publication(1, Some(7), &mut removed)
                .unwrap();
        }
        assert_eq!(table.legacy(&1).unwrap().instance, 7);
        assert!(removed.is_none());
        assert!(!scope.is_cancelled());

        let (occupied, other_task, other_drain) = leaving::tests::running(8);
        removed = Some(Box::new(occupied));
        assert!(table
            .prepare_legacy_publication(1, Some(7), &mut removed)
            .is_err());
        assert_eq!(removed.as_ref().unwrap().instance, 8);
        assert!(!scope.is_cancelled());
        drop(table);
        drop(removed);
        drop(account);
        for task in [task, drain, other_task, other_drain] {
            task.abort();
        }
    }

    #[tokio::test]
    async fn legacy_replacement_returns_old_whole_owner_outside_native_guards() {
        let state = AppState::default();
        let (original, task, drain) = leaving::tests::running(10);
        let (replacement, other_task, other_drain) = leaving::tests::running(11);
        let (account, signal) = watch::channel(false);
        let old_scope = original
            .files
            .scope(RequestCancellation::new(signal.clone(), None));
        let new_scope = replacement
            .files
            .scope(RequestCancellation::new(signal, None));
        let replacement = Box::new(replacement);
        state
            .servers
            .lock()
            .await
            .install_legacy_fixture(1, original);

        // The real owning output is declared before both guards. It is still
        // intact after publication, so no old actor/voice/mesh destructor has
        // been run by the insertion under those guards.
        let mut removed = None;
        let commit = state.ui_session_commit.clone().lock_owned().await;
        let mut table = state.servers.lock().await;
        table
            .prepare_legacy_publication(1, Some(10), &mut removed)
            .unwrap()
            .publish(replacement);
        assert_eq!(table.legacy(&1).unwrap().instance, 11);
        assert_eq!(removed.as_ref().unwrap().instance, 10);
        assert!(
            old_scope.is_cancelled(),
            "retire before returning the old owner"
        );
        assert!(!new_scope.is_cancelled());
        drop(table);
        drop(commit);
        assert!(state.ui_session_commit.try_lock().is_ok());
        assert!(state.servers.try_lock().is_ok());
        drop(removed);
        drop(state);
        drop(account);
        for task in [task, drain, other_task, other_drain] {
            task.abort();
        }
    }

    #[tokio::test]
    async fn native_only_shutdown_cannot_bypass_a_legacy_stop_barrier() {
        let (entry, task, drain) = leaving::tests::running(12);
        let (account, signal) = watch::channel(false);
        let scope = entry.files.scope(RequestCancellation::new(signal, None));
        let mut table = ServerTable::default();
        table.install_legacy_fixture(1, entry);
        let mut removed = None;
        assert!(table.fence_native_for_shutdown().is_err());
        assert!(table.extract_native_for_shutdown(&mut removed).is_err());
        assert!(removed.is_none());
        assert_eq!(table.legacy(&1).unwrap().instance, 12);
        assert!(!scope.is_cancelled());
        drop(table);
        drop(account);
        task.abort();
        drain.abort();
    }
}
