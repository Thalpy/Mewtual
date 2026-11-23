//! Ordinary envelopes from the exact restored founder graph and selected Core.
use super::*;
use crate::owner_v2::{
    open_v2_member_envelope_data, seal_v2_member_envelope_data, v2_member_envelope_topic_data,
    MemberTransportFence, PreparedV2MemberSubmission, SelectedV2MemberEnvelope,
    V2MemberEnvelopeScopeData, V2MemberPublicationObservation, MAX_V2_MEMBER_TRANSPORT_BYTES,
};
use catcoms_replication::chat::V2GeneralPrefixVerifier;
use catcoms_rt::{PublishOnceError, RequestCancellation, Topic};

impl PreparedV2FounderRecoveredAdd {
    /// Bind once after the original mounted founder has matched its restored
    /// graph with its selected signed Head and Core replay.
    pub fn bind_selected_member(
        &mut self,
        verifier: &V2GeneralPrefixVerifier,
    ) -> Result<(), SyncError> {
        let candidate = self.require_selected(verifier)?;
        if let Some(original) = &self.member {
            if !original.same_original(&candidate) {
                return Err(unavailable());
            }
        } else {
            self.member = Some(candidate);
        }
        Ok(())
    }

    fn require_selected(
        &self,
        verifier: &V2GeneralPrefixVerifier,
    ) -> Result<V2AuthenticatedAdmission, SyncError> {
        let replay = verifier.replay_data();
        let original = verifier
            .admission(self.founder)
            .map_err(|_| unavailable())?;
        if replay.header().id() != self.expected.header
            || replay.header().spec().state != self.expected.state
            || replay.mls_epoch_data() != self.expected.epoch
            || replay.profile().spec().founder != self.signer
            || original.admission_data().spec().subject != self.signer
            || !original.is_open_data()
            || !self.group.is_active()
            || self.group.group_id_ref() != replay.profile().spec().group
            || self.group.epoch() != self.expected.epoch
            || self.group.epoch_authenticator_id() != self.expected.authenticator
            || !self.device.retained_ed25519_key_matches(&self.signer)
            || !self.group.has_local_signer(&self.signer)
        {
            return Err(unavailable());
        }
        Ok(original)
    }

    fn observe_selected(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        channel: u128,
        mount: RequestCancellation,
    ) -> Result<(V2MemberEnvelopeScopeData, V2MemberPublicationObservation), SyncError> {
        let original = self.require_selected(verifier)?;
        if mount.is_cancelled()
            || self
                .member
                .as_ref()
                .is_none_or(|member| !member.same_original(&original))
        {
            return Err(unavailable());
        }
        let scope = V2MemberEnvelopeScopeData::for_selected_prefix(verifier, channel)?;
        let observation = V2MemberPublicationObservation {
            member: verifier
                .observe_live_member(self.founder)
                .map_err(|_| unavailable())?,
            fence: MemberTransportFence::Founder {
                seed: Arc::downgrade(&self.lifetime),
                scope: mount,
            },
        };
        Ok((scope, observation))
    }

    pub fn prepare_member_topic(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        channel: u128,
        mount: RequestCancellation,
    ) -> Result<(Topic, V2MemberPublicationObservation), SyncError> {
        let (scope, observation) = self.observe_selected(verifier, channel, mount)?;
        Ok((
            v2_member_envelope_topic_data(&self.group, &self.device, &scope)?,
            observation,
        ))
    }

    pub fn open_member_envelope(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        channel: u128,
        topic: &Topic,
        wire: &[u8],
        mount: RequestCancellation,
    ) -> Result<SelectedV2MemberEnvelope, SyncError> {
        if wire.len() > MAX_V2_MEMBER_TRANSPORT_BYTES {
            return Err(SyncError::Malformed);
        }
        let (scope, observation) = self.observe_selected(verifier, channel, mount)?;
        if *topic != v2_member_envelope_topic_data(&self.group, &self.device, &scope)? {
            return Err(SyncError::Malformed);
        }
        let opened = open_v2_member_envelope_data(&self.group, &self.device, &scope, wire)?;
        let recipient = self.member.as_ref().ok_or_else(unavailable)?.clone();
        Ok(SelectedV2MemberEnvelope {
            opened,
            recipient,
            recipient_id: self.founder,
            observation,
        })
    }

    pub fn prepare_member_submission(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        transport: &impl MeshTransport,
        channel: u128,
        original: &[u8],
        mount: RequestCancellation,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<PreparedV2MemberSubmission, SyncError> {
        let (scope, observation) = self.observe_selected(verifier, channel, mount)?;
        let transport = transport
            .capture_authorized_publication()
            .ok_or(PublishOnceError::Unsupported)?;
        let topic = v2_member_envelope_topic_data(&self.group, &self.device, &scope)?;
        let wire = seal_v2_member_envelope_data(
            &self.group,
            &self.device,
            &scope,
            self.founder,
            original,
            rng,
        )?;
        Ok(PreparedV2MemberSubmission {
            transport,
            topic,
            wire: bytes::Bytes::from(wire),
            observation,
        })
    }
}
