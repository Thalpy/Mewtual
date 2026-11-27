//! Bounded replay of one selected received message and its minimal dependencies.
use super::disk::SharedHistoryObservation;
use super::{SharedHistoryActionQuery, SharedHistoryScope};
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::persistence::v2_member_outbox::MemberSavedOriginal;
use crate::store::persistence::v2_member_transport::LocalDependencyStack;
use crate::store::{ServerStore, V2MemberSaveRefusal};
use crate::AppError;
use catcoms_replication::chat::{
    ActionId, ActionState, ChatLedger, ProjectionView, V2FrozenAdmission, V2GeneralPrefixVerifier,
    V2LocalArchiveOriginal, V2OrdinaryReceiveStatus, MAX_PARENTS,
};
use catcoms_rt::RequestCancellation;

const ACTION_LIMIT: usize = 64;
const LEDGER_WORK: usize = 4 * ACTION_LIMIT + MAX_PARENTS;
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
}

impl SharedHistoryProjectionLease {
    pub(in crate::store) fn with_publication<T>(
        &self,
        publish: impl FnOnce() -> T,
    ) -> Result<T, AppError> {
        self.selected.observation.with_publication(publish)
    }
}

pub(in crate::store) struct SharedHistoryMessageSelection {
    observation: SharedHistoryObservation,
    actions: Vec<ActionId>,
}

impl SharedHistoryMessageSelection {
    pub(in crate::store) fn includes(&self, action: ActionId) -> bool {
        self.actions.binary_search(&action).is_ok()
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
        let page = self
            .store
            .shared_history_action_page(
                self.scope,
                SharedHistoryActionQuery {
                    target: Some(create),
                    after: None,
                    limit: ACTION_LIMIT,
                },
            )
            .map_err(|_| Refusal::Failed)?;
        if page.actions.is_empty() {
            return Ok(None);
        }
        if page.more || page.actions.binary_search(&create).is_err() {
            return Err(Refusal::Failed);
        }
        let roots = page.actions.clone();
        let observation = page.observation.clone();
        let retired = retired.iter().collect::<Vec<_>>();
        drop(page);

        let mut pending = LocalDependencyStack::new(&roots);
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
        observation
            .with_publication(|| ())
            .map_err(|_| Refusal::Stale)?;
        Ok(Some(SharedHistoryMessageSelection {
            observation,
            actions: roots,
        }))
    }
}
