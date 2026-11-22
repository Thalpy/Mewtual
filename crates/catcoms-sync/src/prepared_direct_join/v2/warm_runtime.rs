//! Move the actual private Welcome into one closed joined node.
use super::*;
use crate::owner_v2::{V2JoinFacts, V2JoinedSourceFence};
use catcoms_replication::chat::{
    join_bindings::SignedWelcomeBindingData, owner_facts::OwnerFactId, owner_header::OwnerHeaderId,
};

/// Owns the original verified group, device, routing, runtime and once-taken
/// join facts. The mounted source still owns durable selection and native
/// admission. There is no legacy actor conversion or SDK extraction.
pub struct ClosedV2JoinedNode<T: MeshTransport, R: CryptoRngCore> {
    node: Box<ChannelSync<T, R>>,
    facts: V2JoinFacts,
    fence: V2JoinedSourceFence,
    head: OwnerHeaderId,
    admission: OwnerFactId,
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
        let epoch = self.facts.join_epoch_data();
        if self.node.epoch() != epoch {
            return Err(SyncError::JoinRejected);
        }
        Ok((self.head, self.admission, epoch))
    }
}

impl PreparedV2JoinRequest {
    /// Move the same originally verified Welcome into a closed runtime after
    /// the host has proved its covered selection. All input checks happen
    /// before the one-time move. No historical reader can supply fresh facts.
    pub fn take_closed_warm_node<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        prefix: &PreparedV2JoinedPrefix,
        transport: T,
        rng: R,
        clock: Box<dyn Clock + Send>,
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
        prefix.require_warm_selection(
            original,
            packet.request.spec_data().anchor,
            &welcome,
            graph.state,
            graph.facts_data(),
        )?;

        let graph = self.completed.take().ok_or(SyncError::JoinRejected)?;
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
            facts,
            fence,
            head: OwnerHeaderId::from_bytes(welcome.spec_data().add_header),
            admission,
        })
    }
}
