//! Checked review reads on the joined side of the original founder route.
use super::*;
use crate::owner_v2::{
    PreparedV2ReviewDelivery, ReviewEndpoint, V2ReviewDeliveryData, REVIEW_STREAM_CARRIER,
};
use catcoms_replication::chat::{
    join_bindings::GenesisAnchorBindingData,
    owner_bodies::{ReviewStreamQuery, ReviewStreamRecord},
    FounderGenesisAnchorData,
};

impl<T: MeshTransport, R: CryptoRngCore> ClosedV2JoinedNode<T, R> {
    /// The caller captures the founder peer from the original invite and
    /// retains that exact live route. `contact` is this device's signed
    /// contact for its current Admission and selected epoch.
    pub fn prepare_review_delivery(
        &self,
        prefix: &PreparedV2JoinedPrefix,
        pin: &FounderGenesisAnchorData,
        route: V2MemberDependencyRoute,
        contact: &V2MemberContactData<'_>,
        query: ReviewStreamQuery,
        rng: &mut impl CryptoRngCore,
    ) -> Result<PreparedV2ReviewDelivery, SyncError> {
        let verifier = self.member_prefix(prefix)?;
        if contact.require_review_holder(verifier, route.requester)? != self.admission_id {
            return Err(SyncError::JoinRejected);
        }
        self.review_delivery_endpoint(verifier, pin, route)?
            .prepare(contact, query, rng)
    }

    pub fn open_review_delivery(
        &self,
        prefix: &PreparedV2JoinedPrefix,
        pin: &FounderGenesisAnchorData,
        route: V2MemberDependencyRoute,
        request: &PreparedV2ReviewDelivery,
        wire: &[u8],
    ) -> Result<Option<V2ReviewDeliveryData>, SyncError> {
        let verifier = self.member_prefix(prefix)?;
        let result = self
            .review_delivery_endpoint(verifier, pin, route)?
            .open_reply(pin, request, wire)?;
        match &result {
            Some(V2ReviewDeliveryData::Header(header)) => {
                let selected = header.position_data();
                prefix.require_selected_review_anchor((
                    selected.membership_header,
                    selected.membership_epoch,
                ))?;
            }
            Some(V2ReviewDeliveryData::Approval(bytes)) => {
                let record =
                    ReviewStreamRecord::decode(pin, bytes).map_err(|_| SyncError::JoinRejected)?;
                let selected = record.entry_data().position_data();
                let remove = record.decision_data().original_data();
                prefix.require_selected_review_anchor((
                    selected.membership_header,
                    selected.membership_epoch,
                ))?;
                prefix
                    .require_selected_review_anchor((remove.remove_header, remove.remove_epoch))?;
            }
            None => {}
        }
        Ok(result)
    }

    /// Bind one original governing coordinate to the actual installed
    /// joined node and return its original pinned founder data.
    pub fn require_review_membership_anchor<'a>(
        &self,
        prefix: &'a PreparedV2JoinedPrefix,
        position: (OwnerHeaderId, u64),
    ) -> Result<&'a GenesisAnchorBindingData, SyncError> {
        self.member_prefix(prefix)?;
        prefix.require_selected_review_anchor(position)
    }

    fn review_delivery_endpoint<'a>(
        &'a self,
        verifier: &V2GeneralPrefixVerifier,
        pin: &FounderGenesisAnchorData,
        route: V2MemberDependencyRoute,
    ) -> Result<ReviewEndpoint<'a>, SyncError> {
        let current = verifier.replay_data();
        if route.requester != self.local_peer_data()
            || current.profile().id() != pin.profile
            || current.profile().spec().group != pin.group
            || current.profile().spec().founder != pin.founder
        {
            return Err(SyncError::JoinRejected);
        }
        let exact = GenesisAnchorBindingData {
            group: pin.group.clone(),
            founder: pin.founder,
            profile: *pin.profile.as_bytes(),
            admission: *pin.admission.as_bytes(),
            tenure: *pin.tenure.as_bytes(),
            header: *pin.header.as_bytes(),
            state: *pin.state.as_bytes(),
        }
        .commitment_data()
        .map_err(|_| SyncError::JoinRejected)?;
        verifier
            .require_join_genesis_data(exact)
            .map_err(|_| SyncError::JoinRejected)?;
        let founder = verifier
            .admission(pin.admission)
            .map_err(|_| SyncError::JoinRejected)?;
        if !founder.is_open_data() || founder.admission_data().spec().subject != pin.founder {
            return Err(SyncError::JoinRejected);
        }
        Ok(ReviewEndpoint {
            group: &self.node.group,
            device: &self.node.device,
            scope: V2MemberEnvelopeScopeData::for_selected_prefix(verifier, REVIEW_STREAM_CARRIER)?,
            route,
            founder: pin.admission,
        })
    }
}
