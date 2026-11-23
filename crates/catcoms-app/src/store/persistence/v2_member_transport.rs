//! Internal finite operations on the original selected founder member source.
use super::v2_member_outbox::MemberDebtBatch;
use super::v2_shared_history::{
    SharedHistoryActionPage, SharedHistoryActionQuery, SharedHistoryProjectionLease,
};
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use catcoms_net::NativeChatMemberFrame;
use catcoms_replication::chat::{ActionId, GuardedMessageProjection};
use catcoms_rt::{RequestCancellation, Topic};
use catcoms_sync::owner_v2::{PreparedV2MemberSubmission, V2MemberPublicationObservation};
use std::sync::Arc;

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
}
