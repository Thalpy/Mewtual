//! A paid read over the original invite founder's current connection. The
//! response is verified data for a later durable recipient selection.
use super::dependencies::REPLY_FEE;
use super::*;
use crate::store::{
    persistence::governance_layout::joined::review_receive::{
        pinned_founder, ApprovalSelection, ReviewActionPage, ReviewActionScope, ReviewHeaderPoll,
        SelectedReviewActions, REVIEW_ACTION_CURSOR_BYTES,
    },
    NativeTextInvitePacketData, NativeTextReviewApprovalStatus, NativeTextReviewHeaderPollStatus,
    NativeTextReviewHeaderStatus,
};
use catcoms_net::{
    NativeChatMemberDependencyExchange, NativeChatMemberDependencyObservation,
    NativeChatMemberInventoryExchange,
};
use catcoms_replication::chat::{
    join_bindings::GenesisAnchorBindingData,
    owner_bodies::{ReviewStreamEntryId, ReviewStreamQuery, ReviewStreamRecord},
    FounderGenesisAnchorData, V2ReviewedCreateCheck,
};
use catcoms_rt::TransportError;

#[cfg(test)]
impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    /// Observe only the inert hint through the genuine original receiver.
    /// This supplies no connection, transport handle or authorization.
    pub(in crate::store) fn remembered_founder_peer_for_test(
        &self,
        selection: &NativeTextJoinedRowSelection,
        caller: &RequestCancellation,
    ) -> Result<Option<PeerId>, Refusal> {
        self.with_member_receiver(selection, caller, |source| Ok(source.founder))
    }
}
use catcoms_sync::{
    owner_v2::{
        PreparedV2ReviewDelivery, V2MemberDependencyRoute, V2MemberPublicationObservation,
        V2ReviewDeliveryData,
    },
    V2MemberContactData, V2_MEMBER_CONTACT_BYTES,
};
use std::ops::RangeInclusive;

type Refusal = V2MemberSaveRefusal;

struct ReviewTarget {
    founder: PeerId,
    invite: [u8; 32],
    pin: FounderGenesisAnchorData,
}

impl<R: CryptoRngCore> ReceiverSource<'_, R> {
    fn review_poll_membership(
        &self,
    ) -> Result<(&GenesisAnchorBindingData, (OwnerHeaderId, u64)), Refusal> {
        let verifier = self
            .prefix
            .selected_member_verifier()
            .map_err(|_| Refusal::Stale)?;
        let membership = verifier.replay_data();
        let position = (membership.header().id(), membership.mls_epoch_data());
        let binding = self
            .node
            .require_review_membership_anchor(self.prefix, position)
            .map_err(|_| Refusal::Stale)?;
        Ok((binding, position))
    }
}

fn review_storage_refusal(error: AppError) -> Refusal {
    match error {
        AppError::ChatV2ArchiveBusy => Refusal::Busy,
        AppError::ChatV2ReceiveCapacity => Refusal::ReceiveCapacity,
        _ => Refusal::Failed,
    }
}

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

/// The result of one bounded scan of the original joined channel's selected
/// reviewed receipts. A stale cursor should restart from the first page.
pub enum NativeTextJoinedReviewedPage {
    Unavailable,
    StaleCursor,
    Current(Box<NativeTextJoinedReviewedActions>),
}

pub struct NativeTextJoinedReviewedActions {
    original: NativeTextJoinedRowSelection,
    channel: u128,
    ui: UiEffectNegative,
    caller: RequestCancellation,
    membership: V2MemberPublicationObservation,
    selected: SelectedReviewActions,
}

impl NativeTextJoinedReviewedPage {
    /// Exact ciphertext width for a continuation, before any IPC encoding.
    pub const CURSOR_BYTES: usize = REVIEW_ACTION_CURSOR_BYTES;
}

impl NativeTextJoinedReviewedActions {
    /// Resolve under the native publication guard for the original row and
    /// conversation. An empty slice may have an opaque continuation when
    /// this bounded scan inspected only receipts from other channels.
    pub fn with_publication<T>(
        &self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        publish: impl FnOnce(&[ActionId], Option<&[u8]>) -> T,
    ) -> Result<T, Refusal> {
        self.selected
            .observation
            .with_publication(|| {
                if !self.original.same_original(selection)
                    || self.channel != channel
                    || !self.original.is_current_cached()
                    || !self.ui.is_current_cached()
                    || !self.membership.is_current_cached()
                    || self.caller.is_cancelled()
                {
                    return Err(Refusal::Stale);
                }
                Ok(publish(
                    &self.selected.actions,
                    self.selected.next.as_ref().map(|cursor| cursor.as_slice()),
                ))
            })
            .map_err(|error| match error {
                AppError::ChatV2ArchiveBusy => Refusal::Busy,
                _ => Refusal::Stale,
            })?
    }
}

catcoms_rt::redacted_debug! {
    NativeTextJoinedReviewExchange;
    NativeTextJoinedReviewResponse;
    NativeTextJoinedReviewedPage;
    NativeTextJoinedReviewedActions;
}

impl NativeTextJoinedReviewExchange {
    /// Scheduling identity for an exact approval. It carries no receipt,
    /// channel access or permission to fetch another body.
    pub fn approval_entry_data(&self) -> Option<ReviewStreamEntryId> {
        match self.context.request.query_data() {
            ReviewStreamQuery::Approval { entry, .. } => Some(entry),
            ReviewStreamQuery::Header(_) => None,
        }
    }

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
    /// List durable reviewed IDs with a cursor valid for the exact archive
    /// root and this row's channel. This grants no projection authority.
    pub fn reviewed_action_page(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        after: Option<&[u8]>,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedReviewedPage, Refusal> {
        let (selected, membership) = self.with_member_receiver(selection, caller, |source| {
            let (binding, _) = source.review_poll_membership()?;
            let target = ReviewActionScope {
                row_request: selection.request_data(),
                channel,
            };
            let selected = source
                .store
                .read_selected_review_actions(binding, (target, after), rng)
                .map_err(review_storage_refusal)?;
            if let Some(ReviewActionPage::Selected(page)) = selected.as_ref() {
                for anchor in page.anchors.iter().flatten().copied() {
                    if source
                        .node
                        .require_review_membership_anchor(source.prefix, anchor)
                        .map_err(|_| Refusal::Stale)?
                        != binding
                    {
                        return Err(Refusal::Stale);
                    }
                }
                let (membership, _) = source.effect_data()?;
                Ok((selected, Some(membership)))
            } else {
                Ok((selected, None))
            }
        })?;
        Ok(match selected {
            None => NativeTextJoinedReviewedPage::Unavailable,
            Some(ReviewActionPage::StaleCursor) => NativeTextJoinedReviewedPage::StaleCursor,
            Some(ReviewActionPage::Selected(selected)) => {
                NativeTextJoinedReviewedPage::Current(Box::new(NativeTextJoinedReviewedActions {
                    original: selection.clone(),
                    channel,
                    ui: self.original.command.ui.into_effect_negative(),
                    caller: caller.clone(),
                    membership: membership.ok_or(Refusal::Stale)?,
                    selected,
                }))
            }
        })
    }

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
        let target = ReviewTarget {
            founder: invite.connected_peer_data(),
            invite: invite.binding().id_data(),
            pin: pinned_founder(&invite.binding().spec_data().anchor),
        };
        self.prepare_review_delivery_to(selection, target, (contact, query), caller, rng)
    }

    /// Poll the selected public stream from the exact connected invite
    /// founder. The current native source supplies both the pin and route.
    /// No invite data or new dial is needed after that route is installed.
    pub fn prepare_next_review_header(
        self,
        selection: &NativeTextJoinedRowSelection,
        (contact, check_tip): (&[u8; V2_MEMBER_CONTACT_BYTES], bool),
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<NativeTextJoinedReviewExchange>, Refusal> {
        let plan = self.with_member_receiver(selection, &caller, |source| {
            let Some(founder) = source.founder else {
                return Ok(None);
            };
            let (pin, _) = source.review_poll_membership()?;
            let poll = source
                .store
                .next_received_review_header(pin, check_tip)
                .map_err(review_storage_refusal)?;
            let ReviewHeaderPoll::Query(sequence) = poll else {
                return Ok(None);
            };
            Ok(Some((
                ReviewTarget {
                    founder,
                    invite: source.registration.invite,
                    pin: pinned_founder(pin),
                },
                ReviewStreamQuery::Header(sequence),
            )))
        })?;
        let Some((target, query)) = plan else {
            return Ok(None);
        };
        self.prepare_review_delivery_to(selection, target, (contact, query), caller, rng)
            .map(Some)
    }

    /// Prepare one exact approval request for the caller's already
    /// authorized native row channel. The public header does not disclose
    /// that channel. A body is available only for a matching request.
    /// Start a pass with `None` and keep
    /// the returned cursor before awaiting the exchange. No body reply,
    /// an unavailable ancestor or a lost response leaves the header
    /// available to retry on the next pass without holding this cursor.
    pub fn prepare_next_review_approval(
        self,
        selection: &NativeTextJoinedRowSelection,
        (channel, cursor): (u128, Option<RangeInclusive<u64>>),
        contact: &[u8; V2_MEMBER_CONTACT_BYTES],
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<
        (
            Option<RangeInclusive<u64>>,
            Option<NativeTextJoinedReviewExchange>,
        ),
        Refusal,
    > {
        let (next, plan) = self.with_member_receiver(selection, &caller, |source| {
            let Some(founder) = source.founder else {
                return Ok((cursor, None));
            };
            let (binding, _) = source.review_poll_membership()?;
            let (next, candidate) = source
                .store
                .scan_selected_review_approvals(binding, cursor)
                .map_err(review_storage_refusal)?;
            let Some((position, entry)) = candidate else {
                return Ok((next, None));
            };
            if source
                .node
                .require_review_membership_anchor(
                    source.prefix,
                    (position.membership_header, position.membership_epoch),
                )
                .map_err(|_| Refusal::Stale)?
                != binding
            {
                return Err(Refusal::Stale);
            }
            let target = ReviewTarget {
                founder,
                invite: source.registration.invite,
                pin: pinned_founder(binding),
            };
            let query = ReviewStreamQuery::Approval {
                sequence: position.sequence,
                entry,
                channel,
            };
            Ok((next, Some((target, query))))
        })?;
        let Some((target, query)) = plan else {
            return Ok((next, None));
        };
        match self.prepare_review_delivery_to(selection, target, (contact, query), caller, rng) {
            Ok(exchange) => Ok((next, Some(exchange))),
            // Proven local pressure or no connected founder spends no wire
            // attempt. Preserve this pass's cursor and revisit on wrap.
            Err(Refusal::Busy | Refusal::Unavailable) => Ok((next, None)),
            Err(error) => Err(error),
        }
    }

    /// Check the durable position and connected original founder before
    /// asking native code for a contact signed in this membership epoch.
    pub fn review_header_contact_scope(
        &self,
        selection: &NativeTextJoinedRowSelection,
        check_tip: bool,
        caller: &RequestCancellation,
    ) -> Result<NativeTextReviewHeaderPollStatus, Refusal> {
        use NativeTextReviewHeaderPollStatus as Status;
        let lane = self.dependency_lane().ok();
        self.with_member_receiver(selection, caller, |source| {
            let (binding, position) = source.review_poll_membership()?;
            let poll = source
                .store
                .next_received_review_header(binding, check_tip)
                .map_err(review_storage_refusal)?;
            match poll {
                ReviewHeaderPoll::Frozen => Ok(Status::Frozen),
                ReviewHeaderPoll::Unavailable => Ok(Status::Unavailable),
                ReviewHeaderPoll::Query(_) => {
                    let connected = source
                        .founder
                        .and_then(|peer| lane.as_ref()?.capture_inventory(peer))
                        .is_some();
                    Ok(if connected {
                        Status::Ready(*position.0.as_bytes(), position.1)
                    } else {
                        Status::Unavailable
                    })
                }
            }
        })
    }

    fn prepare_review_delivery_to(
        self,
        selection: &NativeTextJoinedRowSelection,
        ReviewTarget {
            founder,
            invite: invite_id,
            pin,
        }: ReviewTarget,
        (contact, query): (&[u8; V2_MEMBER_CONTACT_BYTES], ReviewStreamQuery),
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedReviewExchange, Refusal> {
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
        self.finish_review_result(context, selection, caller, result)
    }

    /// Cover public signed review metadata from the original connected
    /// response. This receipt does not add or authorize a chat message.
    pub fn select_review_delivery_header(
        &self,
        selection: &NativeTextJoinedRowSelection,
        response: NativeTextJoinedReviewResponse,
        caller: &RequestCancellation,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Option<NativeTextReviewHeaderStatus>, Refusal> {
        let context = &response.context;
        if !matches!(context.request.query_data(), ReviewStreamQuery::Header(_)) {
            return Err(Refusal::Failed);
        }
        context.require_original(selection)?;
        let result = self.with_member_receiver(selection, caller, |source| {
            if source.registration.invite != context.invite
                || source.founder != Some(context.route.provider)
            {
                return Err(Refusal::Stale);
            }
            let data = source
                .node
                .open_review_delivery(
                    source.prefix,
                    &context.pin,
                    context.route,
                    &context.request,
                    &response.wire,
                )
                .map_err(|_| Refusal::Failed)?;
            let incoming = match data {
                Some(V2ReviewDeliveryData::Header(entry)) => entry,
                None => return Ok(None),
                Some(V2ReviewDeliveryData::Approval(_)) => return Err(Refusal::Failed),
            };
            let position = incoming.position_data();
            let binding = source
                .node
                .require_review_membership_anchor(
                    source.prefix,
                    (position.membership_header, position.membership_epoch),
                )
                .map_err(|_| Refusal::Stale)?;
            source
                .store
                .select_received_review_header(
                    binding,
                    &incoming,
                    |position| {
                        context.require_original(selection).map_err(|_| refused())?;
                        self.original.require_current(caller)?;
                        if let Some(position) = position {
                            let actual = source
                                .node
                                .require_review_membership_anchor(
                                    source.prefix,
                                    (position.membership_header, position.membership_epoch),
                                )
                                .map_err(|_| refused())?;
                            if actual != binding {
                                return Err(refused());
                            }
                        }
                        Ok(())
                    },
                    rng,
                )
                .map(Some)
                .map_err(review_storage_refusal)
        })?;
        self.finish_review_result(context, selection, caller, result)
    }

    /// Select the exact approval and original after rechecking the public
    /// case, removed Admission and verified local reply ancestry. The
    /// receipt has its own selector and does not populate the chat ledger.
    pub fn select_review_delivery_approval(
        &self,
        selection: &NativeTextJoinedRowSelection,
        response: NativeTextJoinedReviewResponse,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<NativeTextReviewApprovalStatus>, Refusal> {
        use NativeTextReviewApprovalStatus as Status;
        let context = &response.context;
        let ReviewStreamQuery::Approval {
            sequence,
            entry,
            channel,
        } = context.request.query_data()
        else {
            return Err(Refusal::Failed);
        };
        let require_live = || {
            context.require_original(selection)?;
            self.original
                .require_current(caller)
                .map_err(|_| Refusal::Cancelled)
        };
        require_live()?;
        let result = self.with_member_receiver(selection, caller, |source| {
            if source.registration.invite != context.invite
                || source.founder != Some(context.route.provider)
            {
                return Err(Refusal::Stale);
            }
            let response = source
                .node
                .open_review_delivery(
                    source.prefix,
                    &context.pin,
                    context.route,
                    &context.request,
                    &response.wire,
                )
                .map_err(|_| Refusal::Failed)?;
            let raw = match response {
                None => return Ok(None),
                Some(V2ReviewDeliveryData::Approval(raw)) => raw,
                Some(V2ReviewDeliveryData::Header(_)) => return Err(Refusal::Failed),
            };
            let record = ReviewStreamRecord::decode_public_approval(&context.pin, entry, &raw)
                .map_err(|_| Refusal::Failed)?;
            let coordinates = record.entry_data().position_data();
            let membership = (coordinates.membership_header, coordinates.membership_epoch);
            let target = record.held_data().target_data().spec();
            if coordinates.sequence != sequence || target.channel != channel {
                return Err(Refusal::Failed);
            }
            let binding = source
                .node
                .require_review_membership_anchor(source.prefix, membership)
                .map_err(|_| Refusal::Stale)?;
            let remove = record.held_data().removal_data();
            if source
                .node
                .require_review_membership_anchor(source.prefix, remove)
                .map_err(|_| Refusal::Stale)?
                != binding
            {
                return Err(Refusal::Stale);
            }
            let selected = match source
                .store
                .check_received_review_approval(binding, record.entry_data())
                .map_err(review_storage_refusal)?
            {
                ApprovalSelection::WaitingFor(sequence) => {
                    return Ok(Some(Status::WaitingForHeader(sequence)))
                }
                ApprovalSelection::Frozen => return Ok(Some(Status::Frozen)),
                ApprovalSelection::Selected { header, .. } => header,
            };
            let verifier = source
                .prefix
                .selected_member_verifier()
                .map_err(|_| Refusal::Unavailable)?;
            let retired = source
                .prefix
                .retired_history_data()
                .iter()
                .collect::<Vec<_>>();
            let author = retired
                .iter()
                .copied()
                .find(|author| author.admission_id_data() == target.context.admission)
                .ok_or(Refusal::Unavailable)?;
            // The local dependency walker checks selected ancestry in a
            // live-mode view. The scratch ledger never issues projection
            // tokens or contributes to the ordinary message archive.
            let mut paid = source.fresh_ledger(channel, ReceiverView::Live)?;
            let arguments = (&*selected, &record);
            let scope = (&context.pin, membership);
            let initial = verifier
                .check_selected_reviewed_create(
                    scope,
                    arguments,
                    author,
                    (&paid.ledger, &paid.view),
                    &retired,
                )
                .map_err(|_| Refusal::Failed)?;
            let missing = match initial {
                V2ReviewedCreateCheck::WaitingForReplyParent(parent) => Some(parent),
                V2ReviewedCreateCheck::Ready(_) => None,
            };
            if let Some(parent) = missing {
                replay_local_member_dependencies(
                    std::slice::from_ref(&parent),
                    (verifier, &retired, &mut paid.ledger, &paid.view),
                    RECEIVE_WORK,
                    (source.store, source.history_scope(channel)),
                    |action| source.read_dependency(channel, action, rng),
                    require_live,
                )?;
            }
            let checked = match verifier
                .check_selected_reviewed_create(
                    scope,
                    arguments,
                    author,
                    (&paid.ledger, &paid.view),
                    &retired,
                )
                .map_err(|_| Refusal::Failed)?
            {
                V2ReviewedCreateCheck::WaitingForReplyParent(parent) => {
                    return Ok(Some(Status::WaitingForReplyParent(parent)))
                }
                V2ReviewedCreateCheck::Ready(checked) => checked,
            };
            source
                .store
                .select_received_review_approval(
                    binding,
                    &raw,
                    || {
                        require_live().map_err(|_| refused())?;
                        verifier
                            .require_current_reviewed_create(&checked, &selected)
                            .map_err(|_| refused())
                    },
                    rng,
                )
                .map(Some)
                .map_err(review_storage_refusal)
        })?;
        self.finish_review_result(context, selection, caller, result)
    }

    fn finish_review_result<T>(
        &self,
        context: &Context,
        selection: &NativeTextJoinedRowSelection,
        caller: &RequestCancellation,
        result: T,
    ) -> Result<T, Refusal> {
        context.require_original(selection)?;
        self.original
            .require_current(caller)
            .map_err(|_| Refusal::Cancelled)?;
        Ok(result)
    }
}
