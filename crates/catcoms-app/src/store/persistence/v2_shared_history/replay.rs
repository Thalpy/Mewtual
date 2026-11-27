//! Bounded replay of one selected received message and its minimal dependencies.
use super::{SharedHistoryScope, SharedHistoryTargetSnapshot, MAX_SHARED_HISTORY_RECORDS};
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::store::persistence::v2_member_outbox::MemberSavedOriginal;
use crate::store::persistence::v2_member_transport::LocalDependencyStack;
use crate::store::{ServerStore, V2MemberSaveRefusal};
use crate::AppError;
use catcoms_replication::chat::{
    ActionId, ActionState, ChatLedger, LedgerLimits, ProjectionView, V2FrozenAdmission,
    V2GeneralPrefixVerifier, V2LocalArchiveOriginal, V2OrdinaryReceiveStatus, MAX_ACTION_BYTES,
    MAX_PARENTS,
};
use catcoms_rt::RequestCancellation;

use std::collections::BTreeMap;

const OUTBOX_ANCESTORS: usize = 64;
pub(in crate::store) const ARCHIVED_LEDGER_LIMITS: LedgerLimits = LedgerLimits {
    max_actions: MAX_SHARED_HISTORY_RECORDS + OUTBOX_ANCESTORS,
    max_action_bytes: (MAX_SHARED_HISTORY_RECORDS + OUTBOX_ANCESTORS) * MAX_ACTION_BYTES,
};
const ACTION_LIMIT: usize = ARCHIVED_LEDGER_LIMITS.max_actions;
const LEDGER_WORK: usize = 4 * ACTION_LIMIT + MAX_PARENTS;
const REPLAY_CONTROL: usize = ACTION_LIMIT * 512 + 64 * 1024;
const PER_ORIGINAL_GRAPH: usize = (MAX_PARENTS + 2) * 256;

/// Moved beside and dropped after the replay ledger, even on error. One
/// canonical original can be read twice while its dependencies are resolved,
/// so retain one debit per exact ID. Authenticate each read independently.
#[derive(Default)]
pub(in crate::store) struct SharedHistoryReplayCredit {
    originals: BTreeMap<ActionId, OriginalDebit>,
    local_only: usize,
    control: Option<CatalogReadReservation>,
}

struct OriginalDebit {
    fingerprint: blake3::Hash,
    local_only: bool,
    _fee: CatalogReadReservation,
}

impl SharedHistoryReplayCredit {
    fn begin(&mut self, pool: &CriticalCatalogReadPool) -> Result<(), V2MemberSaveRefusal> {
        if self.control.is_some() {
            return Err(V2MemberSaveRefusal::Failed);
        }
        self.control = Some(
            pool.try_reserve(REPLAY_CONTROL)
                .map_err(|_| V2MemberSaveRefusal::Busy)?,
        );
        Ok(())
    }

    fn retain(
        &mut self,
        action: ActionId,
        original: &[u8],
        local_only: bool,
    ) -> Result<(), V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let bytes = original.len();
        if bytes > MAX_ACTION_BYTES {
            return Err(Refusal::Failed);
        }
        let fingerprint = blake3::hash(original);
        if let Some(previous) = self.originals.get(&action) {
            if (previous.fingerprint, previous.local_only) != (fingerprint, local_only) {
                return Err(Refusal::Stale);
            }
            return Ok(());
        }
        if self.originals.len() == ACTION_LIMIT
            || (local_only && self.local_only == OUTBOX_ANCESTORS)
        {
            return Err(Refusal::Unavailable);
        }
        let invoice = self
            .control
            .as_ref()
            .ok_or(Refusal::Failed)?
            .reserve_more(2 * bytes + PER_ORIGINAL_GRAPH)
            .map_err(|_| Refusal::Busy)?;
        self.originals.insert(
            action,
            OriginalDebit {
                fingerprint,
                local_only,
                _fee: invoice,
            },
        );
        self.local_only += usize::from(local_only);
        Ok(())
    }
}
// A complete archive read can scan every target and examine every parent
// edge of the loaded ledger. Each node is expanded at most once.
pub(in crate::store) const ARCHIVED_PROJECTION_WORK: usize = (1 + MAX_PARENTS) * ACTION_LIMIT;

/// The archive revision, projection view and ledger stay owned until final UI
/// publication. A joined view also retains its original invocation sender.
pub(in crate::store) struct SharedHistoryProjectionLease {
    pub(in crate::store) selected: SharedHistoryMessageSelection,
    pub(in crate::store) _view: ProjectionView,
    pub(in crate::store) _ledger: ChatLedger,
    pub(in crate::store) _invocation: Option<tokio::sync::watch::Sender<bool>>,
    pub(in crate::store) _fee: CatalogReadReservation,
    pub(in crate::store) _replay: SharedHistoryReplayCredit,
}

impl SharedHistoryProjectionLease {
    pub(in crate::store) fn with_publication<T>(
        &self,
        publish: impl FnOnce() -> T,
    ) -> Result<T, AppError> {
        self.selected.snapshot.observation.with_publication(publish)
    }
}

pub(in crate::store) struct SharedHistoryMessageSelection {
    snapshot: SharedHistoryTargetSnapshot,
}

impl SharedHistoryMessageSelection {
    pub(in crate::store) fn includes(&self, action: ActionId) -> bool {
        self.snapshot.actions.binary_search(&action).is_ok()
    }
}

pub(in crate::store) struct SharedHistoryReplay<'a> {
    pub(in crate::store) store: &'a ServerStore,
    pub(in crate::store) scope: SharedHistoryScope,
    pub(in crate::store) verifier: &'a V2GeneralPrefixVerifier,
    pub(in crate::store) caller: &'a RequestCancellation,
}

impl SharedHistoryReplay<'_> {
    pub(in crate::store) fn replay_message(
        self,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        create: ActionId,
        retired: &[V2FrozenAdmission],
        credit: &mut SharedHistoryReplayCredit,
        mut read_local: impl FnMut(ActionId) -> Result<Option<MemberSavedOriginal>, V2MemberSaveRefusal>,
    ) -> Result<Option<SharedHistoryMessageSelection>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !self
            .store
            .has_mounted_shared_history()
            .map_err(|_| Refusal::Busy)?
        {
            return Ok(None);
        }
        let snapshot = self
            .store
            .shared_history_target_snapshot(self.scope, create)
            .map_err(|error| match error {
                AppError::ChatV2ArchiveBusy => Refusal::Busy,
                _ => Refusal::Failed,
            })?;
        if snapshot.actions.is_empty() {
            return Ok(None);
        }
        let roots = &snapshot.actions;
        if roots.binary_search(&create).is_err() {
            return Err(Refusal::Failed);
        }
        let pool = &self
            .store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or(Refusal::Unavailable)?
            .pending
            .pool;
        credit.begin(pool)?;
        let retired = retired.iter().collect::<Vec<_>>();
        let mut pending = LocalDependencyStack::exact_sorted(roots, ACTION_LIMIT)?;
        let mut reads = 2 * ACTION_LIMIT;
        while let Some((action, retry)) = pending.0.pop() {
            if self.caller.is_cancelled() {
                return Err(Refusal::Cancelled);
            }
            if ledger.state(action) == Some(ActionState::Valid) {
                continue;
            }
            // One charged read per visit or retry. A duplicate queued parent
            // must not spend the work needed to verify a selected revision.
            reads = reads.checked_sub(1).ok_or(Refusal::Unavailable)?;
            let archived = self
                .store
                .read_selected_shared_history_record(self.scope, action)
                .map_err(|_| Refusal::Failed)?;
            let status = if let Some(original) = archived {
                let signed = original.original.action();
                let target = signed.spec().body.create().unwrap_or(action);
                if roots.binary_search(&action).is_ok() && target != create {
                    return Err(Refusal::Failed);
                }
                let selected = V2LocalArchiveOriginal::from_trusted_local_selection(
                    signed,
                    original.admission,
                    original.original.original_bytes(),
                )
                .map_err(|_| Refusal::Failed)?;
                credit.retain(action, original.original.original_bytes(), false)?;
                self.verifier.project_local_archive_original(
                    ledger,
                    view,
                    selected,
                    &retired,
                    LEDGER_WORK,
                )
            } else {
                if roots.binary_search(&action).is_ok() {
                    return Err(Refusal::Failed);
                }
                let original = read_local(action)?.ok_or(Refusal::Unavailable)?;
                credit.retain(action, original.original_bytes(), true)?;
                self.verifier.replay_local_archive_dependency(
                    ledger,
                    view,
                    original.action(),
                    original.receipt().admission,
                    &retired,
                    LEDGER_WORK,
                )
            }
            .map_err(|_| Refusal::Failed)?;
            match status {
                V2OrdinaryReceiveStatus::Retained(ActionState::Valid) => {}
                V2OrdinaryReceiveStatus::Retained(_) => return Err(Refusal::Failed),
                V2OrdinaryReceiveStatus::WaitingForDependencies(missing) => {
                    if retry {
                        return Err(Refusal::Unavailable);
                    }
                    pending.descend(action, &missing, |parent| {
                        ledger.state(parent) == Some(ActionState::Valid)
                    })?;
                }
            }
        }
        if self.caller.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        snapshot
            .observation
            .with_publication(|| ())
            .map_err(|_| Refusal::Stale)?;
        Ok(Some(SharedHistoryMessageSelection { snapshot }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_archive_debit_refuses_pressure_and_releases_on_drop() {
        use V2MemberSaveRefusal as Refusal;
        let total = REPLAY_CONTROL + PER_ORIGINAL_GRAPH + 2 * 3;
        let pool = CriticalCatalogReadPool::new(total).unwrap();
        let mut replay = SharedHistoryReplayCredit::default();
        replay.begin(&pool).unwrap();
        let id = ActionId::from_bytes([1; 32]);
        replay.retain(id, b"one", false).unwrap();
        assert_eq!(pool.charged_for_test(), total);
        replay.retain(id, b"one", false).unwrap();
        assert_eq!(replay.retain(id, b"two", false), Err(Refusal::Stale));
        assert_eq!(replay.retain(id, b"one", true), Err(Refusal::Stale));
        assert_eq!(
            replay.retain(ActionId::from_bytes([2; 32]), b"two", false),
            Err(Refusal::Busy),
        );
        assert_eq!(pool.charged_for_test(), total);
        drop(replay);
        assert_eq!(pool.charged_for_test(), 0);
    }
}
