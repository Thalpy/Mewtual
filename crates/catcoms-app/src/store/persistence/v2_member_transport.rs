//! Internal finite operations on the original selected founder member source.
use super::v2_member_outbox::{member_outbox_key, MemberDebtBatch, MemberSavedOriginal};
use super::v2_shared_history::{
    PreparedSharedHistoryRecord, SharedHistoryActionPage, SharedHistoryActionQuery,
    SharedHistoryProjectionLease, SharedHistoryScope, SHARED_HISTORY_RECORD_FEE,
};
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::critical_namespaces::refused;
use crate::store::{NativeTextJoinedMemberProjection, ServerStore, V2MemberSaveRefusal as Refusal};
use crate::AppError;
use catcoms_net::NativeChatMemberFrame;
use catcoms_replication::chat::{
    ActionId, ActionState, ChatLedger, GuardedMessageProjection, ProjectionView,
    V2GeneralPrefixVerifier, V2OrdinaryReceiveStatus, MAX_PARENTS,
};
use catcoms_rt::{CryptoRngCore, RequestCancellation, Topic};
use catcoms_sync::owner_v2::{
    PreparedV2MemberSubmission, SelectedV2MemberEnvelope, V2MemberPublicationObservation,
};
use std::sync::Arc;

pub(in crate::store) const FOUNDER_RECEIVE_ACTIONS: usize = 64;
pub(in crate::store) const FOUNDER_RECEIVE_BYTES: usize = 1024 * 1024;
pub(in crate::store) const FOUNDER_RECEIVE_WORK: usize = 4 * FOUNDER_RECEIVE_ACTIONS + MAX_PARENTS;
pub(in crate::store) const FOUNDER_RECEIVE_LEDGER_FEE: usize = 2 * FOUNDER_RECEIVE_BYTES
    + FOUNDER_RECEIVE_ACTIONS * (MAX_PARENTS + 2) * 256
    + V2GeneralPrefixVerifier::ordinary_authorization_temporary_layout_bytes()
    + 16 * 1024;

/// Project a live envelope, then retry once with exact saved, authenticated
/// direct dependencies. The storage owner chooses which originals can be lent.
pub(in crate::store) fn project_founder_member(
    original: &SelectedV2MemberEnvelope,
    verifier: &V2GeneralPrefixVerifier,
    ledger: &mut ChatLedger,
    view: &ProjectionView,
    mut saved_dependency: impl FnMut(ActionId) -> Result<Option<MemberSavedOriginal>, Refusal>,
    mut require_current: impl FnMut() -> Result<(), Refusal>,
) -> Result<bool, Refusal> {
    require_current()?;
    let mut status = original
        .project_from_ledger(verifier, ledger, view, FOUNDER_RECEIVE_WORK)
        .map_err(|_| Refusal::Failed)?;
    if let V2OrdinaryReceiveStatus::WaitingForDependencies(missing) = &status {
        if missing.len() > 4 {
            return Ok(false);
        }
        let mut replayed = false;
        for &id in missing {
            require_current()?;
            let Some(saved) = saved_dependency(id)? else {
                continue;
            };
            let admission = verifier
                .admission(saved.receipt().admission)
                .map_err(|_| Refusal::Stale)?;
            require_current()?;
            replayed |= verifier
                .replay_ordinary_history_original(
                    ledger,
                    view,
                    saved.action(),
                    &admission,
                    &[],
                    FOUNDER_RECEIVE_WORK,
                )
                .map_err(|_| Refusal::Failed)?
                == V2OrdinaryReceiveStatus::Retained(ActionState::Valid);
        }
        if !replayed {
            return Ok(false);
        }
        status = original
            .project_from_ledger(verifier, ledger, view, FOUNDER_RECEIVE_WORK)
            .map_err(|_| Refusal::Failed)?;
    }
    require_current()?;
    Ok(status == V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
}

pub(in crate::store) fn archive_founder_member(
    store: &ServerStore,
    scope: SharedHistoryScope,
    original: &SelectedV2MemberEnvelope,
    rng: &mut impl CryptoRngCore,
    require_current: impl FnOnce() -> Result<(), AppError>,
) -> Result<Option<bool>, AppError> {
    if !store.has_mounted_shared_history()? {
        return Ok(None);
    }
    let _fee = store
        .archive_bootstrap_policy
        .as_ref()
        .ok_or_else(refused)?
        .pending
        .pool
        .try_reserve(SHARED_HISTORY_RECORD_FEE)?;
    let key = member_outbox_key(store)?;
    let record = PreparedSharedHistoryRecord::prepare(
        &scope,
        &key,
        original.admission_data(),
        original.action_data(),
        original.original_data(),
        rng,
    )?;
    let bundle = record.encode_bundle()?;
    require_current()?;
    store.save_shared_history_record(scope, record.action, record.manifest.id(), &bundle, rng)?;
    Ok(Some(true))
}

pub(in crate::store) enum FounderDebtOperation {
    Prepare,
    Checkpoint(MemberDebtBatch),
}

pub(in crate::store) enum FounderDebtOutcome {
    Idle,
    Prepared {
        submission: PreparedV2MemberSubmission,
        batch: MemberDebtBatch,
        fee: CatalogReadReservation,
    },
    Checkpointed(u64),
}

#[derive(Clone)]
pub(in crate::store) struct FounderReceiveSession {
    pub(in crate::store) identity: Arc<()>,
    pub(in crate::store) signal: RequestCancellation,
}

pub(in crate::store) enum FounderReceiveOperation<'a> {
    Subscription,
    Frame(&'a NativeChatMemberFrame, &'a FounderReceiveSession),
    Read(ActionId),
    ArchivePage(SharedHistoryActionQuery),
    ArchiveMessage(ActionId),
}

/// The original founder receiver's update for a verified action.
pub struct NativeTextFounderReceived {
    pub action: ActionId,
    pub projection_target: ActionId,
    /// Whether the selected archive covered the action. No value means this
    /// vault has no mounted shared history namespace.
    pub archived: Option<bool>,
}

pub(in crate::store) struct FounderArchivedMessage {
    pub(in crate::store) projection: GuardedMessageProjection,
    pub(in crate::store) signal: RequestCancellation,
    pub(in crate::store) lease: Box<SharedHistoryProjectionLease>,
    pub(in crate::store) fee: CatalogReadReservation,
}

pub(in crate::store) enum FounderReceiveOutcome {
    Subscription(
        Topic,
        V2MemberPublicationObservation,
        CatalogReadReservation,
    ),
    Frame(Option<NativeTextFounderReceived>),
    Read(
        Option<(
            GuardedMessageProjection,
            RequestCancellation,
            CatalogReadReservation,
        )>,
    ),
    ArchivePage(Option<SharedHistoryActionPage>),
    ArchiveMessage(Option<FounderArchivedMessage>),
    RecoveredProjection(Option<NativeTextJoinedMemberProjection>),
}
