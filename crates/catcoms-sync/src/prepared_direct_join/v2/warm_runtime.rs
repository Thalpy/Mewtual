//! Move the actual private Welcome into one closed joined node.
use super::first_add_proof::SelectedJoinData;
use super::*;

pub type V2JoinedRuntimeInputs<T, R> = (T, R, Box<dyn Clock + Send>);
use crate::owner_v2::{
    open_v2_member_envelope_data, seal_v2_member_envelope_data, v2_member_envelope_topic_data,
    JoinFactsSlot, JoinedFenceNode, MemberTransportFence, OpenedV2MemberDependencyRequest,
    PreparedV2MemberDependencyRequest, PreparedV2MemberSubmission, SelectedV2MemberEnvelope,
    V2JoinFacts, V2JoinedFenceAdmission, V2JoinedSourceFence, V2MemberDependencyRoute,
    V2MemberDraftInputs, V2MemberEnvelopeScopeData, V2MemberPublicationObservation,
    V2MemberSignRefusal,
};
use catcoms_replication::chat::{
    join_bindings::SignedWelcomeBindingData, owner_facts::OwnerFactId, owner_header::OwnerHeaderId,
    owner_profile::AdmissionOrigin, ActionDraft, ActionId, V2AuthenticatedAdmission,
    V2GeneralCandidate, V2GeneralPrefixVerifier, V2OrdinarySigningGuard, V2OrdinarySigningPlan,
};

/// Owns the original verified group, device, routing and runtime. Only a
/// warm completed Welcome owns fresh facts. The mounted source still owns
/// durable selection and native admission.
pub struct ClosedV2JoinedNode<T: MeshTransport, R: CryptoRngCore> {
    node: Box<ChannelSync<T, R>>,
    origin: JoinedOrigin,
    fence: V2JoinedSourceFence,
    head: OwnerHeaderId,
    admission: V2AuthenticatedAdmission,
    admission_id: OwnerFactId,
    native_origin: Option<NativeChatSourceObservation>,
}

enum JoinedOrigin {
    Warm(V2JoinFacts),
    Recovered(u64),
}

impl<T: MeshTransport, R: CryptoRngCore> ClosedV2JoinedNode<T, R> {
    /// Initial runtime wrapper overlap, excluding existing private MLS graph,
    /// transport and clock backing funded by the caller.
    pub fn allocation_bytes() -> usize {
        3 * std::mem::size_of::<ChannelSync<T, R>>()
            + std::mem::size_of::<Self>()
            + std::mem::size_of::<T>()
            + 64 * 1024
    }

    pub fn selected_join_data(&self) -> Result<(OwnerHeaderId, OwnerFactId, u64), SyncError> {
        if !self.admission.is_open_data() {
            return Err(SyncError::JoinRejected);
        }
        self.fence
            .require_authorized(self.admission_id.as_bytes())
            .map_err(|_| SyncError::JoinRejected)?;
        let epoch = match &self.origin {
            JoinedOrigin::Warm(facts) => facts.join_epoch_data(),
            JoinedOrigin::Recovered(epoch) => *epoch,
        };
        if self.node.epoch() != epoch {
            return Err(SyncError::JoinRejected);
        }
        Ok((self.head, self.admission_id, epoch))
    }

    /// Hold the real installed node's final fence while the host issues its
    /// original save-source end. Host custody must also cover the original
    /// selected prefix through issuance. This guard performs no SDK or IO.
    pub fn admit_member_save<'a>(
        &'a self,
        plan: &V2OrdinarySigningPlan,
    ) -> Result<V2JoinedFenceAdmission<'a>, SyncError> {
        if plan.admission_id_data() != self.admission_id {
            return Err(SyncError::JoinRejected);
        }
        let admitted = self
            .fence
            .admit_final(self.admission_id.as_bytes())
            .map_err(|_| SyncError::JoinRejected)?;
        plan.require_original_selected_admission(&self.admission)
            .map_err(|_| SyncError::JoinRejected)?;
        Ok(admitted)
    }

    /// Draw this request's time and nonce on the original selected runtime.
    /// The slot retains the draw across retries. Root supplies current Source
    /// admission and must hold exclusive custody of this node.
    pub fn draw_member_draft_inputs(
        &mut self,
        inputs: &mut V2MemberDraftInputs,
    ) -> Result<(u64, [u8; 16]), V2MemberSignRefusal> {
        self.selected_join_data()
            .map_err(|_| V2MemberSignRefusal::Source)?;
        let instance = self.node.registry_instance();
        self.node.draw_ordinary_draft_inputs(&instance, inputs)
    }

    pub fn member_signer_data(&self) -> Result<[u8; 32], V2MemberSignRefusal> {
        self.selected_join_data()
            .map_err(|_| V2MemberSignRefusal::Source)?;
        self.node
            .ordinary_member_signer_data(&self.node.registry_instance())
    }

    /// Arm the restored graph only under its exact selected Core Admission
    /// and the new, currently authenticated host's original native Source.
    /// The native host must have restored its own registered physical row.
    pub fn bind_recovered_member_source(
        &mut self,
        prefix: &PreparedV2JoinedPrefix,
        original: &NativeChatSourceObservation,
        cut: &NativeChatPreparedKeyCut,
    ) -> Result<(), SyncError> {
        if !self.was_recovered_data() {
            return Err(SyncError::JoinRejected);
        }
        self.member_prefix(prefix)?;
        cut.preflight(original, NativeChatKeyPurpose::MemberAction)
            .map_err(|_| SyncError::JoinRejected)?;
        if let Some(bound) = self.native_origin.as_ref() {
            return bound
                .same_original(original)
                .then_some(())
                .ok_or(SyncError::JoinRejected);
        }
        self.native_origin = Some(original.clone());
        Ok(())
    }

    fn member_prefix<'a>(
        &self,
        prefix: &'a PreparedV2JoinedPrefix,
    ) -> Result<&'a V2GeneralPrefixVerifier, SyncError> {
        let verifier = prefix.selected_member_verifier()?;
        let expected = verifier
            .admission(self.admission_id)
            .map_err(|_| SyncError::JoinRejected)?;
        if !self.admission.same_original(&expected) {
            return Err(SyncError::JoinRejected);
        }
        self.selected_join_data()?;
        Ok(verifier)
    }

    /// Prepare the Core successor for a later Add on this exact joined node.
    /// Core records fork evidence while the host prepares a separate MLS
    /// candidate. The original MLS group stays at its selected epoch.
    pub fn prepare_member_add_commit(
        &self,
        prefix: &mut PreparedV2JoinedPrefix,
        step: V2AdmissionStepData<'_>,
        commit: &[u8],
    ) -> Result<V2GeneralCandidate, SyncError> {
        use catcoms_replication::chat::join_bindings::MAX_JOIN_PAYLOAD_BYTES;
        self.member_prefix(prefix)?;
        let AdmissionOrigin::Add {
            commit: expected,
            before_epoch,
            after_epoch,
            ..
        } = step.admission.spec().origin
        else {
            return Err(SyncError::JoinRejected);
        };
        let epoch = self.node.epoch();
        if commit.is_empty()
            || commit.len() > MAX_JOIN_PAYLOAD_BYTES
            || before_epoch != epoch
            || before_epoch.checked_add(1) != Some(after_epoch)
            || blake3::hash(commit).as_bytes() != &expected
        {
            return Err(SyncError::JoinRejected);
        }
        catcoms_mls::group::check_commit_header(
            commit,
            &step.admission.spec().group,
            before_epoch,
            epoch,
        )
        .map_err(|_| SyncError::JoinRejected)?;
        prefix.prepare_member_successor(step)
    }

    pub fn selected_member_topic_data(
        &self,
        prefix: &PreparedV2JoinedPrefix,
        channel: u128,
    ) -> Result<catcoms_rt::Topic, SyncError> {
        let scope =
            V2MemberEnvelopeScopeData::for_selected_prefix(self.member_prefix(prefix)?, channel)?;
        v2_member_envelope_topic_data(&self.node.group, &self.node.device, &scope)
    }

    /// Current encrypted attempt for the host's exact saved own Admission.
    /// App still owns durable debt and must separately authorize the transport.
    pub fn seal_selected_member_envelope(
        &mut self,
        prefix: &PreparedV2JoinedPrefix,
        channel: u128,
        admission: OwnerFactId,
        original: &[u8],
    ) -> Result<Vec<u8>, SyncError> {
        if admission != self.admission_id {
            return Err(SyncError::JoinRejected);
        }
        let scope =
            V2MemberEnvelopeScopeData::for_selected_prefix(self.member_prefix(prefix)?, channel)?;
        let node = &mut *self.node;
        seal_v2_member_envelope_data(
            &node.group,
            &node.device,
            &scope,
            admission,
            original,
            &mut node.rng,
        )
    }

    pub fn open_selected_member_envelope(
        &self,
        prefix: &PreparedV2JoinedPrefix,
        channel: u128,
        topic: &catcoms_rt::Topic,
        wire: &[u8],
    ) -> Result<SelectedV2MemberEnvelope, SyncError> {
        if wire.len() > crate::owner_v2::MAX_V2_MEMBER_TRANSPORT_BYTES {
            return Err(SyncError::Malformed);
        }
        let observation = self.observe_member_publication(prefix)?;
        let scope =
            V2MemberEnvelopeScopeData::for_selected_prefix(self.member_prefix(prefix)?, channel)?;
        if *topic != v2_member_envelope_topic_data(&self.node.group, &self.node.device, &scope)? {
            return Err(SyncError::Malformed);
        }
        let opened =
            open_v2_member_envelope_data(&self.node.group, &self.node.device, &scope, wire)?;
        Ok(SelectedV2MemberEnvelope {
            opened,
            recipient: self.admission.clone(),
            recipient_id: self.admission_id,
            observation,
        })
    }

    pub fn prepare_member_dependency_request(
        &self,
        prefix: &PreparedV2JoinedPrefix,
        channel: u128,
        route: V2MemberDependencyRoute,
        action: ActionId,
        rng: &mut impl CryptoRngCore,
    ) -> Result<PreparedV2MemberDependencyRequest, SyncError> {
        if route.requester != self.local_peer_data() {
            return Err(SyncError::JoinRejected);
        }
        let scope =
            V2MemberEnvelopeScopeData::for_selected_prefix(self.member_prefix(prefix)?, channel)?;
        PreparedV2MemberDependencyRequest::prepare(
            &self.node.group,
            &self.node.device,
            scope,
            route,
            action,
            rng,
        )
    }

    pub fn open_member_dependency_request(
        &self,
        prefix: &PreparedV2JoinedPrefix,
        channel: u128,
        route: V2MemberDependencyRoute,
        wire: &[u8],
    ) -> Result<OpenedV2MemberDependencyRequest, SyncError> {
        if route.provider != self.local_peer_data() {
            return Err(SyncError::JoinRejected);
        }
        let scope =
            V2MemberEnvelopeScopeData::for_selected_prefix(self.member_prefix(prefix)?, channel)?;
        OpenedV2MemberDependencyRequest::open(
            &self.node.group,
            &self.node.device,
            scope,
            route,
            wire,
        )
    }

    pub fn seal_member_dependency_reply(
        &self,
        prefix: &PreparedV2JoinedPrefix,
        channel: u128,
        route: V2MemberDependencyRoute,
        request: &OpenedV2MemberDependencyRequest,
        saved: Option<(OwnerFactId, &[u8])>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<u8>, SyncError> {
        if route.provider != self.local_peer_data()
            || saved.is_some_and(|(id, _)| id != self.admission_id)
        {
            return Err(SyncError::JoinRejected);
        }
        let scope =
            V2MemberEnvelopeScopeData::for_selected_prefix(self.member_prefix(prefix)?, channel)?;
        request.seal_reply(
            &self.node.group,
            &self.node.device,
            &scope,
            route,
            saved,
            rng,
        )
    }

    pub fn open_member_dependency_reply(
        &self,
        prefix: &PreparedV2JoinedPrefix,
        channel: u128,
        route: V2MemberDependencyRoute,
        request: &PreparedV2MemberDependencyRequest,
        wire: &[u8],
    ) -> Result<Option<SelectedV2MemberEnvelope>, SyncError> {
        if route.requester != self.local_peer_data() {
            return Err(SyncError::JoinRejected);
        }
        let observation = self.observe_member_publication(prefix)?;
        let scope =
            V2MemberEnvelopeScopeData::for_selected_prefix(self.member_prefix(prefix)?, channel)?;
        let opened =
            request.open_reply(&self.node.group, &self.node.device, &scope, route, wire)?;
        Ok(opened.map(|opened| SelectedV2MemberEnvelope {
            opened,
            recipient: self.admission.clone(),
            recipient_id: self.admission_id,
            observation,
        }))
    }

    /// Prepare one paid host-selected original for the actual mesh driver.
    /// Hold the outer native admission and payment through lower ownership.
    pub fn prepare_member_submission(
        &mut self,
        prefix: &PreparedV2JoinedPrefix,
        channel: u128,
        admission: OwnerFactId,
        original: &[u8],
    ) -> Result<PreparedV2MemberSubmission, SyncError> {
        if admission != self.admission_id {
            return Err(SyncError::JoinRejected);
        }
        let observation = self.observe_member_publication(prefix)?;
        let transport = self
            .node
            .transport()
            .capture_authorized_publication()
            .ok_or(catcoms_rt::PublishOnceError::Unsupported)?;
        let topic = self.selected_member_topic_data(prefix, channel)?;
        let wire = self.seal_selected_member_envelope(prefix, channel, admission, original)?;
        Ok(PreparedV2MemberSubmission {
            transport,
            topic,
            wire: bytes::Bytes::from(wire),
            observation,
        })
    }

    pub fn observe_member_publication(
        &self,
        prefix: &PreparedV2JoinedPrefix,
    ) -> Result<V2MemberPublicationObservation, SyncError> {
        let member = self
            .member_prefix(prefix)?
            .observe_live_member(self.admission_id)
            .map_err(|_| SyncError::JoinRejected)?;
        let fence = self
            .fence
            .observe_publication(self.admission_id.as_bytes())
            .map_err(|_| SyncError::JoinRejected)?;
        Ok(V2MemberPublicationObservation {
            member,
            fence: MemberTransportFence::Joined(fence),
        })
    }

    /// Sign using this warm native joiner's original device and Source. The
    /// caller must hold the mounted prefix/source custody and keep its native
    /// cut and ends outside the enclosing writers. No historical graph can
    /// manufacture the original native issuer. A restored graph needs its own
    /// separately bound native Source before it can use this cut.
    pub fn sign_member_action_native<'a>(
        &self,
        inputs: &V2MemberDraftInputs,
        draft: &'a ActionDraft,
        plan: &'a mut V2OrdinarySigningPlan,
        original: &NativeChatSourceObservation,
        cut: &mut NativeChatPreparedKeyCut,
        ends: &mut NativeChatKeyEnds,
    ) -> Result<V2OrdinarySigningGuard<'a>, V2MemberSignRefusal> {
        if !self
            .native_origin
            .as_ref()
            .is_some_and(|source| source.same_original(original))
        {
            return Err(V2MemberSignRefusal::Source);
        }
        let signer = self
            .node
            .ordinary_member_signer_data(&self.node.registry_instance())?;
        let _fence = self
            .admit_member_save(plan)
            .map_err(|_| V2MemberSignRefusal::Source)?;
        let guard = plan
            .admit_signing(draft, &signer)
            .map_err(|_| V2MemberSignRefusal::Source)?;
        self.node
            .sign_ordinary_admitted_native(inputs, guard, original, cut, ends)
    }

    pub fn was_recovered_data(&self) -> bool {
        matches!(self.origin, JoinedOrigin::Recovered(_))
    }

    pub fn local_peer_data(&self) -> catcoms_rt::PeerId {
        self.node.local_peer()
    }

    pub(super) fn from_exact_recovered_graph(
        mut node: ChannelSync<T, R>,
        head: OwnerHeaderId,
        admission: V2AuthenticatedAdmission,
        epoch: u64,
        authenticator: [u8; 32],
    ) -> Result<Self, SyncError> {
        if node.epoch() != epoch || node.group.epoch_authenticator_id() != authenticator {
            return Err(SyncError::JoinRejected);
        }
        // The independently proved restored graph supplies the settlement.
        // There is no post-Welcome handoff or fresh join fact to take.
        node.v2_fence = JoinedFenceNode::for_join(&node.group);
        node.v2_join_facts = JoinFactsSlot::TakenByJoinedSource;
        let fence = node.v2_fence.handle().ok_or(SyncError::JoinRejected)?;
        let admission_id = admission.admission_data().admission_id();
        fence
            .install_selected(epoch, authenticator, *admission_id.as_bytes())
            .map_err(|_| SyncError::JoinRejected)?;
        Ok(Self {
            node: Box::new(node),
            origin: JoinedOrigin::Recovered(epoch),
            fence,
            head,
            admission,
            admission_id,
            native_origin: None,
        })
    }
}

impl PreparedV2JoinRequest {
    /// Move the same originally verified Welcome into a closed runtime after
    /// the host has proved its covered selection. All input checks happen
    /// before the one-time move. No historical reader can supply fresh facts.
    pub fn take_closed_warm_node<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        prefix: &PreparedV2JoinedPrefix,
        inputs: &mut Option<V2JoinedRuntimeInputs<T, R>>,
    ) -> Result<ClosedV2JoinedNode<T, R>, SyncError> {
        let original = self
            .initial_prefix_origin
            .as_ref()
            .ok_or(SyncError::JoinRejected)?;
        let graph = self.completed.as_ref().ok_or(SyncError::JoinRejected)?;
        graph.validate_initial_state_data()?;
        let packet = V2JoinRequestPacketData::decode(&self.request)?;
        let welcome = SignedWelcomeBindingData::decode(&graph.response_binding)
            .map_err(|_| SyncError::JoinRejected)?;
        let facts = graph.facts_data();
        let admission = prefix.require_join_selection(
            original,
            SelectedJoinData {
                anchor: packet.request.spec_data().anchor,
                welcome: &welcome,
                state: graph.state,
                signer: facts.signer_data(),
                key_package: facts.key_package_data(),
                credential: facts.credential_data(),
                epoch: facts.join_epoch_data(),
            },
        )?;

        let (transport, rng, clock) = inputs.take().ok_or(SyncError::JoinRejected)?;
        let graph = self.completed.take().expect("checked original warm graph");
        let routing = RoutingState {
            label: 0,
            secrets: vec![(0, graph.routing_secret)],
            policy: Some(graph.policy),
            v2_join_facts: Some(graph.facts),
            ..RoutingState::default()
        };
        let mut node =
            ChannelSync::new_joined(transport, graph.group, graph.device, rng, clock, routing);
        node.file_wrap_key = graph.file_key;
        let instance = node.registry_instance();
        let (facts, fence) = node
            .take_v2_joined_join_facts(&instance)
            .map_err(|_| SyncError::JoinRejected)?;
        let admission_id = prefix.own_admission_data();
        fence
            .install_selected(
                facts.join_epoch_data(),
                *facts.admission_state_data(),
                *admission_id.as_bytes(),
            )
            .map_err(|_| SyncError::JoinRejected)?;
        Ok(ClosedV2JoinedNode {
            node: Box::new(node),
            origin: JoinedOrigin::Warm(facts),
            fence,
            head: OwnerHeaderId::from_bytes(welcome.spec_data().add_header),
            admission,
            admission_id,
            native_origin: self.native_origin.clone(),
        })
    }
}
