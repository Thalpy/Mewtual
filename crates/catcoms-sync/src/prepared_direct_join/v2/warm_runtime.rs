//! Move the actual private Welcome into one closed joined node.
use super::first_add_proof::SelectedJoinData;
use super::*;

pub type V2JoinedRuntimeInputs<T, R> = (T, R, Box<dyn Clock + Send>);
use crate::owner_v2::{JoinFactsSlot, JoinedFenceNode, V2JoinFacts, V2JoinedSourceFence};
use catcoms_replication::chat::{
    join_bindings::SignedWelcomeBindingData, owner_facts::OwnerFactId, owner_header::OwnerHeaderId,
};

/// Owns the original verified group, device, routing and runtime. Only a
/// warm completed Welcome owns fresh facts. The mounted source still owns
/// durable selection and native admission.
pub struct ClosedV2JoinedNode<T: MeshTransport, R: CryptoRngCore> {
    node: Box<ChannelSync<T, R>>,
    origin: JoinedOrigin,
    fence: V2JoinedSourceFence,
    head: OwnerHeaderId,
    admission: OwnerFactId,
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
        self.fence
            .require_authorized(self.admission.as_bytes())
            .map_err(|_| SyncError::JoinRejected)?;
        let epoch = match &self.origin {
            JoinedOrigin::Warm(facts) => facts.join_epoch_data(),
            JoinedOrigin::Recovered(epoch) => *epoch,
        };
        if self.node.epoch() != epoch {
            return Err(SyncError::JoinRejected);
        }
        Ok((self.head, self.admission, epoch))
    }

    pub fn was_recovered_data(&self) -> bool {
        matches!(self.origin, JoinedOrigin::Recovered(_))
    }

    pub(super) fn from_exact_recovered_graph(
        mut node: ChannelSync<T, R>,
        head: OwnerHeaderId,
        admission: OwnerFactId,
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
        fence
            .install_selected(epoch, authenticator, *admission.as_bytes())
            .map_err(|_| SyncError::JoinRejected)?;
        Ok(Self {
            node: Box::new(node),
            origin: JoinedOrigin::Recovered(epoch),
            fence,
            head,
            admission,
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
        prefix.require_join_selection(
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
        let admission = prefix.own_admission_data();
        fence
            .install_selected(
                facts.join_epoch_data(),
                *facts.admission_state_data(),
                *admission.as_bytes(),
            )
            .map_err(|_| SyncError::JoinRejected)?;
        Ok(ClosedV2JoinedNode {
            node: Box::new(node),
            origin: JoinedOrigin::Warm(facts),
            fence,
            head: OwnerHeaderId::from_bytes(welcome.spec_data().add_header),
            admission,
        })
    }
}
