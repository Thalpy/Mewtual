//! Ordinary traffic from the original installed founder's MLS pair.
use super::*;
use crate::owner_v2::{
    open_v2_member_envelope_data, seal_v2_member_envelope_data, v2_member_envelope_topic_data,
    MemberTransportFence, PreparedV2MemberSubmission, SelectedV2MemberEnvelope,
    V2FounderMemberDependencyAccess, V2MemberEnvelopeScopeData, V2MemberPublicationObservation,
    MAX_V2_MEMBER_TRANSPORT_BYTES,
};
use crate::V2MemberAddRequestData;
use catcoms_replication::chat::{owner_facts::OwnerFactId, V2GeneralPrefixVerifier};
use catcoms_rt::{PublishOnceError, Topic};

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2FounderGovernanceNode<T, R> {
    /// Capture the request verifier while the original predecessor SDK pair
    /// is still selected. A later epoch uses an independent exporter secret.
    pub fn member_add_request_data(
        &self,
        verifier: &V2GeneralPrefixVerifier,
    ) -> Result<V2MemberAddRequestData, SyncError> {
        self.observe_member(verifier, 0)?;
        let seed = &self.node.seed.body;
        V2MemberAddRequestData::for_selected_group(
            &seed.group,
            &seed.device,
            verifier.replay_data(),
        )
    }

    fn observe_member(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        channel: u128,
    ) -> Result<(V2MemberEnvelopeScopeData, V2MemberPublicationObservation), SyncError> {
        self.require_sdk_current_data()?;
        let installed = self.installed.ok_or_else(unavailable)?;
        let recipient = self.member.as_ref().ok_or_else(unavailable)?;
        let id = recipient.admission_data().admission_id();
        let scope = V2MemberEnvelopeScopeData::for_selected_prefix(verifier, channel)?;
        if !recipient.same_original(&verifier.admission(id).map_err(|_| unavailable())?)
            || scope.governing != installed.header
            || scope.epoch != installed.epoch
            || verifier.replay_data().header().spec().state != installed.state
        {
            return Err(unavailable());
        }
        let seed = &self.node.seed.body;
        let observation = V2MemberPublicationObservation {
            member: verifier
                .observe_live_member(id)
                .map_err(|_| unavailable())?,
            fence: MemberTransportFence::Founder {
                seed: Arc::downgrade(&seed.identity),
                scope: seed.metadata.scope.genesis_attachment_negative()?,
            },
        };
        Ok((scope, observation))
    }

    pub fn select_member_dependency_access(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        channel: u128,
    ) -> Result<V2FounderMemberDependencyAccess<'_>, SyncError> {
        let (scope, observation) = self.observe_member(verifier, channel)?;
        let seed = &self.node.seed.body;
        let member = self.member.as_ref().ok_or_else(unavailable)?;
        Ok(V2FounderMemberDependencyAccess {
            group: &seed.group,
            device: &seed.device,
            scope,
            member,
            local: self.node._parts.body._transport.local_peer(),
            observation,
        })
    }

    /// Topic and retirement fence for the founder's original selected group.
    pub fn prepare_member_subscription(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        channel: u128,
    ) -> Result<(Topic, V2MemberPublicationObservation), SyncError> {
        let (scope, observation) = self.observe_member(verifier, channel)?;
        let seed = &self.node.seed.body;
        let topic = v2_member_envelope_topic_data(&seed.group, &seed.device, &scope)?;
        Ok((topic, observation))
    }

    /// Seal an exact own original for the retained mesh. The caller still
    /// owns the selected outbox debt and its native submission authority.
    pub fn prepare_member_submission(
        &mut self,
        verifier: &V2GeneralPrefixVerifier,
        channel: u128,
        admission: OwnerFactId,
        original: &[u8],
    ) -> Result<PreparedV2MemberSubmission, SyncError> {
        if self
            .member
            .as_ref()
            .map(|member| member.admission_data().admission_id())
            != Some(admission)
        {
            return Err(unavailable());
        }
        let (scope, observation) = self.observe_member(verifier, channel)?;
        let runtime = &mut self.node._parts.body;
        let transport = runtime
            ._transport
            .capture_authorized_publication()
            .ok_or(PublishOnceError::Unsupported)?;
        let seed = &self.node.seed.body;
        let topic = v2_member_envelope_topic_data(&seed.group, &seed.device, &scope)?;
        let wire = seal_v2_member_envelope_data(
            &seed.group,
            &seed.device,
            &scope,
            admission,
            original,
            &mut runtime._rng,
        )?;
        Ok(PreparedV2MemberSubmission {
            transport,
            topic,
            wire: bytes::Bytes::from(wire),
            observation,
        })
    }

    pub fn open_selected_member_envelope(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        channel: u128,
        topic: &Topic,
        wire: &[u8],
    ) -> Result<SelectedV2MemberEnvelope, SyncError> {
        if wire.len() > MAX_V2_MEMBER_TRANSPORT_BYTES {
            return Err(SyncError::Malformed);
        }
        let (scope, observation) = self.observe_member(verifier, channel)?;
        let seed = &self.node.seed.body;
        if *topic != v2_member_envelope_topic_data(&seed.group, &seed.device, &scope)? {
            return Err(SyncError::Malformed);
        }
        let opened = open_v2_member_envelope_data(&seed.group, &seed.device, &scope, wire)?;
        let recipient = self.member.as_ref().ok_or_else(unavailable)?.clone();
        Ok(SelectedV2MemberEnvelope {
            opened,
            recipient_id: recipient.admission_data().admission_id(),
            recipient,
            observation,
        })
    }
}
