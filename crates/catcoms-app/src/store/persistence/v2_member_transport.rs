//! Internal finite operations on the original selected founder member source.
use super::v2_member_outbox::MemberDebtBatch;
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
}

pub(in crate::store) enum FounderReceiveOutcome {
    Subscription(
        Topic,
        V2MemberPublicationObservation,
        CatalogReadReservation,
    ),
    Frame(Option<ActionId>),
    Read(
        Option<(
            GuardedMessageProjection,
            RequestCancellation,
            CatalogReadReservation,
        )>,
    ),
}
