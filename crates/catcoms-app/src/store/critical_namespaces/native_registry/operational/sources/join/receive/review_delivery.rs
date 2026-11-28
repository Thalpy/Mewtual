//! A paid read over the original invite founder's current connection. The
//! response is verified data for a later durable recipient selection.
use super::dependencies::REPLY_FEE;
use super::*;
use crate::store::NativeTextInvitePacketData;
use catcoms_net::{
    NativeChatMemberDependencyExchange, NativeChatMemberDependencyObservation,
    NativeChatMemberInventoryExchange,
};
use catcoms_replication::chat::{
    owner_bodies::ReviewStreamQuery,
    owner_header::{OwnerStateCommitment, OwnerTenureId},
    FounderGenesisAnchorData,
};
use catcoms_rt::TransportError;
use catcoms_sync::{
    owner_v2::{PreparedV2ReviewDelivery, V2MemberDependencyRoute, V2ReviewDeliveryData},
    V2MemberContactData, V2_MEMBER_CONTACT_BYTES,
};

type Refusal = V2MemberSaveRefusal;

enum Carrier {
    Header(NativeChatMemberInventoryExchange),
    Approval(NativeChatMemberDependencyExchange),
}

struct Context {
    selected: NativeTextJoinedRowSelection,
    invite: [u8; 32],
    pin: FounderGenesisAnchorData,
    route: V2MemberDependencyRoute,
    request: PreparedV2ReviewDelivery,
    observation: NativeChatMemberDependencyObservation,
    _invocation: watch::Sender<bool>,
    signal: RequestCancellation,
}

impl Context {
    fn require_original(&self, selection: &NativeTextJoinedRowSelection) -> Result<(), Refusal> {
        if !selection.same_original(&self.selected)
            || !self.selected.is_current_cached()
            || !self.observation.is_current_cached()
            || self.signal.is_cancelled()
        {
            return Err(Refusal::Stale);
        }
        Ok(())
    }
}

pub struct NativeTextJoinedReviewExchange {
    carrier: Carrier,
    context: Context,
}

pub struct NativeTextJoinedReviewResponse {
    // The retained response drops before its original fee can be released.
    wire: bytes::Bytes,
    context: Context,
}

catcoms_rt::redacted_debug! {
    NativeTextJoinedReviewExchange;
    NativeTextJoinedReviewResponse;
}

impl NativeTextJoinedReviewExchange {
    pub async fn receive(self) -> Result<NativeTextJoinedReviewResponse, TransportError> {
        let Self { carrier, context } = self;
        let frame = context.request.wire_data();
        let signal = context.signal.clone();
        let wire = match carrier {
            Carrier::Header(route) => route.request(frame, signal).await?,
            Carrier::Approval(route) => route.request(frame, signal).await?,
        };
        context
            .require_original(&context.selected)
            .map_err(|_| TransportError::Cancelled)?;
        Ok(NativeTextJoinedReviewResponse { context, wire })
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    #[cfg(test)]
    pub(in crate::store) fn require_review_anchor_for_test(
        &self,
        selection: &NativeTextJoinedRowSelection,
        position: (OwnerHeaderId, u64),
        caller: &RequestCancellation,
    ) -> Result<(), Refusal> {
        self.with_member_receiver(selection, caller, |source| {
            source
                .node
                .require_review_membership_anchor(source.prefix, position)
                .map(|_| ())
                .map_err(|_| Refusal::Failed)
        })
    }

    pub fn prepare_review_delivery(
        self,
        selection: &NativeTextJoinedRowSelection,
        invite: &NativeTextInvitePacketData<'_>,
        (contact, query): (&[u8; V2_MEMBER_CONTACT_BYTES], ReviewStreamQuery),
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedReviewExchange, Refusal> {
        let founder = invite.connected_peer_data();
        let lane = self.dependency_lane()?;
        let (carrier, observation) = match query {
            ReviewStreamQuery::Header(_) => {
                let exchange = lane
                    .capture_inventory(founder)
                    .ok_or(Refusal::Unavailable)?;
                let observation = exchange.observe_route();
                (Carrier::Header(exchange), observation)
            }
            ReviewStreamQuery::Approval { .. } => {
                let exchange = lane.capture_request(founder).ok_or(Refusal::Unavailable)?;
                let observation = exchange.observe_route();
                (Carrier::Approval(exchange), observation)
            }
        };
        let invite_id = invite.binding().id_data();
        let anchor = &invite.binding().spec_data().anchor;
        let pin = FounderGenesisAnchorData {
            group: anchor.group.clone(),
            founder: anchor.founder,
            profile: OwnerFactId::from_bytes(anchor.profile),
            admission: OwnerFactId::from_bytes(anchor.admission),
            tenure: OwnerTenureId::from_bytes(anchor.tenure),
            header: OwnerHeaderId::from_bytes(anchor.header),
            state: OwnerStateCommitment::from_bytes(anchor.state),
        };
        let (request, route, observed, auth, fee) =
            self.with_member_receiver(selection, &caller, |source| {
                if source.registration.invite != invite_id
                    || source.founder != Some(founder)
                    || !observation.is_current_cached()
                {
                    return Err(Refusal::Stale);
                }
                let fee = source
                    .metadata
                    .reserve_join_node(REPLY_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let route = V2MemberDependencyRoute {
                    requester: source.node.local_peer_data(),
                    provider: founder,
                };
                let contact = V2MemberContactData::decode(contact).map_err(|_| Refusal::Failed)?;
                let request = source
                    .node
                    .prepare_review_delivery(source.prefix, &pin, route, &contact, query, rng)
                    .map_err(|_| Refusal::Failed)?;
                let (observed, auth) = source.effect_data()?;
                Ok((request, route, observed, auth, fee))
            })?;
        let (signal, invocation) = self.dependency_signal(selection, caller, auth, observed);
        let signal = signal.retain_custody(Arc::new(JoinFramePayment { _fee: fee }));
        let context = Context {
            selected: selection.clone(),
            invite: invite_id,
            pin,
            route,
            request,
            observation,
            signal,
            _invocation: invocation,
        };
        context.require_original(selection)?;
        Ok(NativeTextJoinedReviewExchange { carrier, context })
    }

    /// Decode while the original admission, founder route and native row
    /// are live. The result itself grants no authority to add a chat item.
    pub fn with_review_delivery<T>(
        &self,
        selection: &NativeTextJoinedRowSelection,
        response: NativeTextJoinedReviewResponse,
        caller: &RequestCancellation,
        read: impl FnOnce(Option<&V2ReviewDeliveryData>) -> Result<T, Refusal>,
    ) -> Result<T, Refusal> {
        let context = &response.context;
        context.require_original(selection)?;
        let data = self.with_member_receiver(selection, caller, |source| {
            if source.registration.invite != context.invite
                || source.founder != Some(context.route.provider)
            {
                return Err(Refusal::Stale);
            }
            source
                .node
                .open_review_delivery(
                    source.prefix,
                    &context.pin,
                    context.route,
                    &context.request,
                    &response.wire,
                )
                .map_err(|_| Refusal::Failed)
        })?;
        context.require_original(selection)?;
        let result = read(data.as_ref())?;
        context.require_original(selection)?;
        self.original
            .require_current(caller)
            .map_err(|_| Refusal::Cancelled)?;
        Ok(result)
    }
}
