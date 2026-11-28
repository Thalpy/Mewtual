//! Bind passive discovery to the closed source that owns the current MLS
//! graph. The attached host supplies the real connection endpoints, its
//! already signed local proof and the network work budget.
use super::*;
use crate::owner_v2::{
    DiscoveredV2Member, DiscoveryEndpoint, OpenedV2MemberDiscovery, PreparedV2MemberDiscovery,
};

impl<T: MeshTransport, R: CryptoRngCore> ClosedV2JoinedNode<T, R> {
    /// Prepare a challenge on an existing connection after the host checks
    /// its passive discovery policy. The random ciphertext nonce binds the
    /// response to this exact request.
    pub fn prepare_member_discovery(
        &self,
        prefix: &PreparedV2JoinedPrefix,
        route: V2MemberDependencyRoute,
        contact: &V2MemberContactData<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<PreparedV2MemberDiscovery, SyncError> {
        self.member_discovery_endpoint(prefix, route, route.requester)?
            .prepare(contact, rng)
    }

    /// Observe a requesting member after validating its proof on the actual
    /// authenticated route. Unverified requests never need to sign or reply.
    pub fn open_member_discovery_request(
        &self,
        prefix: &PreparedV2JoinedPrefix,
        route: V2MemberDependencyRoute,
        wire: &[u8],
    ) -> Result<OpenedV2MemberDiscovery, SyncError> {
        self.member_discovery_endpoint(prefix, route, route.provider)?
            .open_request(wire)
    }

    /// Seal the existing local proof for this exact validated request. The
    /// native caller retains the original responder through delivery.
    pub fn seal_member_discovery_reply(
        &self,
        prefix: &PreparedV2JoinedPrefix,
        route: V2MemberDependencyRoute,
        request: &OpenedV2MemberDiscovery,
        contact: &V2MemberContactData<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<u8>, SyncError> {
        self.member_discovery_endpoint(prefix, route, route.provider)?
            .seal_reply(request, contact, rng)
    }

    /// Verify the respondent proof on the same request and selected graph.
    /// The host rechecks connection and row before acting on the result.
    pub fn open_member_discovery_reply(
        &self,
        prefix: &PreparedV2JoinedPrefix,
        route: V2MemberDependencyRoute,
        request: &PreparedV2MemberDiscovery,
        wire: &[u8],
    ) -> Result<DiscoveredV2Member, SyncError> {
        self.member_discovery_endpoint(prefix, route, route.requester)?
            .open_reply(request, wire)
    }

    fn member_discovery_endpoint<'a>(
        &'a self,
        prefix: &'a PreparedV2JoinedPrefix,
        route: V2MemberDependencyRoute,
        local: catcoms_rt::PeerId,
    ) -> Result<DiscoveryEndpoint<'a>, SyncError> {
        if local != self.local_peer_data() {
            return Err(SyncError::JoinRejected);
        }
        Ok(DiscoveryEndpoint {
            group: &self.node.group,
            device: &self.node.device,
            verifier: self.member_prefix(prefix)?,
            route,
            local: self.admission_id,
        })
    }
}
