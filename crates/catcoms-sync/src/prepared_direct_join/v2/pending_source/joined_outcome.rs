//! The completed private Join graph, tied to the same sealed request.
use super::super::first_add_proof::SelectedJoinData;
use super::super::welcome::{matches_welcome_roster, open_routing};
use super::*;
use catcoms_mls::persist::{
    restore_server_text_current_bounded, snapshot_server_bounded, TextCurrentSdkRestoreAdmission,
};
use catcoms_replication::chat::{
    owner_header::{OwnerHeaderId, OwnerStateCommitment},
    owner_profile::AdmissionOrigin,
    owner_state::StateFrame,
    ProjectionGroupView, V2HostSelectionExpectation, V2ProjectionOrigin,
};

const TAG: &[u8] = b"catcoms/private-chat-v2-joined-outcome/v1\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-v2-joined-outcome-key/v1";
const PARENT_DOMAIN: &str = "catcoms/private-chat-v2-joined-outcome-parent/v1";
const MAX_POLICY_BYTES: usize = 1024;
const OVERHEAD: usize = 4 + TAG.len() + 2 + 6 * 36 + 8 + 5 * 4 + 2 * 32;
const MAX_ENCRYPTED: usize = OVERHEAD + MAX_JOIN_BINDING_BYTES + MAX_POLICY_BYTES + MAX_MASTER + 16;

pub struct SealedV2JoinedOutcome {
    blob: SealedBlob,
}

impl SealedV2JoinedOutcome {
    /// The mounted reader owns the independent allowance for this exact copy.
    pub fn from_encrypted_parts(nonce: [u8; 24], ciphertext: &[u8]) -> Result<Self, SyncError> {
        if !(OVERHEAD + 17..=MAX_ENCRYPTED).contains(&ciphertext.len()) {
            return Err(SyncError::Malformed);
        }
        Ok(Self {
            blob: SealedBlob {
                nonce,
                ciphertext: ciphertext.to_vec(),
            },
        })
    }

    pub fn encrypted_parts(&self) -> (&[u8; 24], &[u8]) {
        (&self.blob.nonce, &self.blob.ciphertext)
    }
}

pub(super) fn parent_data(source: &SealedV2PendingJoinSource) -> [u8; 32] {
    let (nonce, ciphertext) = source.encrypted_parts();
    let mut parent = blake3::Hasher::new_derive_key(PARENT_DOMAIN);
    parent.update(nonce);
    parent.update(ciphertext);
    *parent.finalize().as_bytes()
}

impl PreparedV2JoinRequest {
    /// Prepare the initial governing transition beside the real retained
    /// Welcome graph. The mounted owner must still check the exact held
    /// Pending, pin and successful Outcome before advancing it.
    pub fn prepare_initial_prefix(
        &mut self,
        delivery: &[u8],
        group: &ProjectionGroupView,
        origin: V2ProjectionOrigin,
    ) -> Result<(PreparedV2JoinedPrefix, V2HostSelectionExpectation), SyncError> {
        if self.sealed_outcome.is_none() || self.initial_prefix_origin.is_some() {
            return Err(SyncError::JoinRejected);
        }
        let graph = self.completed.as_ref().ok_or(SyncError::JoinRejected)?;
        graph.validate_initial_state_data()?;
        self.validate_first_join_delivery_data(delivery)?;
        let packet = V2JoinRequestPacketData::decode(&self.request)?;
        let delivery = V2FirstJoinDeliveryData::decode(delivery)?;
        let (response, proof) = delivery.data();
        if response.binding.encode() != graph.response_binding {
            return Err(SyncError::JoinRejected);
        }
        let selection = V2HostSelectionExpectation::from_trusted_host(
            OwnerHeaderId::from_bytes(response.binding.spec_data().add_header),
            graph.state,
        );
        let mut prepared =
            proof.prepare_joined_prefix(&packet.invite.spec_data().anchor, group, origin)?;
        let original = std::sync::Arc::new(());
        prepared.bind_source_origin(&original);
        self.initial_prefix_origin = Some(original);
        Ok((prepared, selection))
    }

    /// Seal the exact original successful Welcome before joined-source
    /// installation. The host funds the independent snapshot and output
    /// buffers, and covers the returned frame under the mounted journal.
    pub fn seal_joined_outcome(
        &mut self,
        key: &[u8; 32],
        expected: V2PendingJoinContextData,
        rng: &mut impl CryptoRngCore,
    ) -> Result<&SealedV2JoinedOutcome, SyncError> {
        let key_id = *blake3::keyed_hash(key, super::TAG).as_bytes();
        let source = self
            .sealed_source
            .as_ref()
            .filter(|source| source.original == Some((expected, key_id)))
            .ok_or(SyncError::JoinRejected)?;
        let candidate = self.completed.as_ref().ok_or(SyncError::JoinRejected)?;
        candidate.validate_initial_state_data()?;
        if self.sealed_outcome.is_none() {
            self.sealed_outcome = Some(seal_outcome(key, source, candidate, self.limits, rng)?);
        }
        self.sealed_outcome.as_ref().ok_or(SyncError::JoinRejected)
    }
}

fn seal_outcome(
    key: &[u8; 32],
    source: &SealedV2PendingJoinSource,
    candidate: &CompletedV2JoinCandidate,
    limits: DirectJoinLimits,
    rng: &mut impl CryptoRngCore,
) -> Result<SealedV2JoinedOutcome, SyncError> {
    let snapshot = snapshot_server_bounded(
        &candidate.device,
        &candidate.group,
        limits.provider.min(MAX_MASTER),
        limits.backing,
    )?;
    let parent = parent_data(source);
    let policy = candidate.policy_data().encode();
    if policy.len() > MAX_POLICY_BYTES || candidate.response_binding.len() > MAX_JOIN_BINDING_BYTES
    {
        return Err(SyncError::Malformed);
    }
    let length = OVERHEAD + policy.len() + candidate.response_binding.len() + snapshot.len();
    let facts = candidate.facts_data();
    let mut e = Encoder::with_capacity(length);
    let malformed = |_| SyncError::Malformed;
    e.put_bytes(TAG).map_err(malformed)?;
    e.put_u16(VERSION);
    for field in [
        &parent,
        facts.key_package_data(),
        facts.credential_data(),
        facts.admission_state_data(),
        candidate.state.as_bytes(),
        facts.signer_data(),
    ] {
        e.put_bytes(field).map_err(malformed)?;
    }
    e.put_u64(facts.join_epoch_data());
    e.put_bytes(&candidate.response_binding)
        .map_err(malformed)?;
    e.put_bytes(&policy).map_err(malformed)?;
    let mut plain = Zeroizing::new(e.finish());
    for body in [
        &candidate.routing_secret[..],
        &candidate.file_key[..],
        snapshot.as_slice(),
    ] {
        plain.extend_from_slice(&(body.len() as u32).to_be_bytes());
        plain.extend_from_slice(body);
    }
    if plain.len() != length {
        return Err(SyncError::Malformed);
    }
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let blob = seal(&key, &plain, rng).map_err(|_| SyncError::Malformed)?;
    Ok(SealedV2JoinedOutcome { blob })
}

/// Independently authenticated historical state. No Welcome entry or live
/// source is granted by opening the encrypted journal record.
pub struct RecoveredV2JoinedOutcome {
    graph: Option<Box<RecoveredJoinGraph>>,
    head: OwnerHeaderId,
    state: OwnerStateCommitment,
    epoch: u64,
    accepted: GenesisAnchorBindingData,
    authenticator: [u8; 32],
    signer: [u8; 32],
    key_package: [u8; 32],
    credential: [u8; 32],
    welcome: Vec<u8>,
    delivery: [u8; 32],
    prefix_origin: Option<std::sync::Arc<()>>,
}

struct RecoveredJoinGraph {
    device: MlsDevice,
    group: ServerGroup,
    routing_secret: Zeroizing<[u8; 32]>,
    file_key: Zeroizing<[u8; 32]>,
    policy: GroupPolicy,
}

impl RecoveredV2JoinedOutcome {
    pub fn open(
        sealed: &SealedV2JoinedOutcome,
        key: &[u8; 32],
        pending: &RecoveredV2PendingJoinSource,
        delivery_bytes: &[u8],
        paid: TextCurrentSdkRestoreAdmission,
    ) -> Result<Self, SyncError> {
        let limits = pending.limits;
        if delivery_bytes.len() > limits.exchange {
            return Err(SyncError::Malformed);
        }
        let packet = V2JoinRequestPacketData::decode(pending.request_bytes_data())?;
        let delivery = V2FirstJoinDeliveryData::decode(delivery_bytes)?;
        let (response, proof) = delivery.data();
        let anchor = &packet.invite.spec_data().anchor;
        let (profile, admission, header) = proof.verified_admission(anchor)?;
        response
            .binding
            .check_links_data(&packet.invite, &packet.request, profile, admission, header)
            .map_err(|_| SyncError::JoinRejected)?;
        let state =
            StateFrame::decode(proof.add_image_data()).map_err(|_| SyncError::JoinRejected)?;
        let AdmissionOrigin::Add {
            before_epoch: 0,
            after_epoch: 1,
            ..
        } = admission.spec().origin
        else {
            return Err(SyncError::JoinRejected);
        };

        let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
        let plain = Zeroizing::new(unseal(&key, &sealed.blob).map_err(|_| SyncError::Malformed)?);
        let mut d = Decoder::new(&plain);
        let malformed = |_| SyncError::Malformed;
        if d.get_bytes().map_err(malformed)? != TAG || d.get_u16().map_err(malformed)? != VERSION {
            return Err(SyncError::Malformed);
        }
        let request = packet.request.spec_data();
        for expected in [
            &pending.outcome_parent,
            &request.key_package_reference,
            &request.credential,
        ] {
            if d.get_bytes().map_err(malformed)? != expected {
                return Err(SyncError::JoinRejected);
            }
        }
        let authenticator: &[u8; 32] = d
            .get_bytes()
            .map_err(malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        if d.get_bytes().map_err(malformed)? != header.spec().state.as_bytes()
            || d.get_bytes().map_err(malformed)? != request.joiner
        {
            return Err(SyncError::JoinRejected);
        }
        let epoch = d.get_u64().map_err(malformed)?;
        let binding = d.get_bytes().map_err(malformed)?;
        let policy = d.get_bytes().map_err(malformed)?;
        let routing = d.get_bytes().map_err(malformed)?;
        let file = d.get_bytes().map_err(malformed)?;
        let snapshot = d.get_bytes().map_err(malformed)?;
        d.finish().map_err(malformed)?;
        if epoch != 1
            || binding != response.binding.encode()
            || policy.len() > MAX_POLICY_BYTES
            || routing.len() != 32
            || file.len() != 32
            || snapshot.is_empty()
        {
            return Err(SyncError::JoinRejected);
        }

        let (device, group) = restore_server_text_current_bounded(
            snapshot,
            limits.provider.min(MAX_MASTER),
            limits.backing,
            paid,
        )?;
        if group.group_id_ref() != anchor.group
            || !device.retained_ed25519_key_matches(&request.joiner)
            || group.own_credential_digest(&device)? != request.credential
            || group.epoch() != epoch
            || &group.epoch_authenticator_id() != authenticator
            || !matches_welcome_roster(&group, &state)
        {
            return Err(SyncError::JoinRejected);
        }
        let routing_key = Zeroizing::new(group.routing_transfer_key(&device)?);
        let transfer = Zeroizing::new(
            unseal(&routing_key, &decode_sealed(response.sealed_routing)?)
                .map_err(|_| SyncError::JoinRejected)?,
        );
        let (verified_policy, routing_secret, file_key) =
            open_routing(&transfer, &packet, 0, 1, header.spec().parent.as_bytes())?;
        verified_policy.verify_current_owner(&group)?;
        if verified_policy.encode() != policy
            || &routing_secret[..] != routing
            || &file_key[..] != file
        {
            return Err(SyncError::JoinRejected);
        }
        Ok(Self {
            graph: Some(Box::new(RecoveredJoinGraph {
                device,
                group,
                routing_secret,
                file_key,
                policy: verified_policy,
            })),
            head: header.id(),
            state: header.spec().state,
            epoch,
            accepted: anchor.clone(),
            authenticator: *authenticator,
            signer: request.joiner,
            key_package: request.key_package_reference,
            credential: request.credential,
            welcome: response.binding.encode(),
            delivery: *blake3::hash(delivery_bytes).as_bytes(),
            prefix_origin: None,
        })
    }

    pub fn governing_head_data(&self) -> OwnerHeaderId {
        self.head
    }

    pub fn governing_state_data(&self) -> OwnerStateCommitment {
        self.state
    }

    pub fn group_data(&self) -> &[u8] {
        &self.accepted.group
    }

    pub fn epoch_data(&self) -> u64 {
        self.epoch
    }

    /// Reconstruct the real selected-history candidate from this outcome's
    /// exact signed delivery. The App supplies the original paid projection
    /// group and negative origin for its private runtime.
    pub fn prepare_initial_prefix(
        &mut self,
        delivery: &[u8],
        group: &ProjectionGroupView,
        origin: V2ProjectionOrigin,
    ) -> Result<PreparedV2JoinedPrefix, SyncError> {
        if self.prefix_origin.is_some()
            || delivery.len() > MAX_V2_FIRST_JOIN_DELIVERY_BYTES
            || blake3::hash(delivery).as_bytes() != &self.delivery
        {
            return Err(SyncError::JoinRejected);
        }
        let delivery = V2FirstJoinDeliveryData::decode(delivery)?;
        let (_, proof) = delivery.data();
        let mut prepared = proof.prepare_joined_prefix(&self.accepted, group, origin)?;
        let original = std::sync::Arc::new(());
        prepared.bind_source_origin(&original);
        self.prefix_origin = Some(original);
        Ok(prepared)
    }

    /// Move this same recovered graph after the host has proved its selected
    /// storage. Its own original Core prefix must already be selected.
    pub fn take_closed_recovered_node<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        prefix: &PreparedV2JoinedPrefix,
        inputs: &mut Option<V2JoinedRuntimeInputs<T, R>>,
    ) -> Result<ClosedV2JoinedNode<T, R>, SyncError> {
        let original = self.prefix_origin.as_ref().ok_or(SyncError::JoinRejected)?;
        let graph = self.graph.as_ref().ok_or(SyncError::JoinRejected)?;
        let welcome = catcoms_replication::chat::join_bindings::SignedWelcomeBindingData::decode(
            &self.welcome,
        )
        .map_err(|_| SyncError::JoinRejected)?;
        if graph.group.epoch() != self.epoch
            || graph.group.epoch_authenticator_id() != self.authenticator
            || !graph.device.retained_ed25519_key_matches(&self.signer)
            || graph.group.own_credential_digest(&graph.device)? != self.credential
        {
            return Err(SyncError::JoinRejected);
        }
        prefix.require_join_selection(
            original,
            SelectedJoinData {
                anchor: self
                    .accepted
                    .commitment_data()
                    .map_err(|_| SyncError::JoinRejected)?,
                welcome: &welcome,
                state: self.state,
                signer: &self.signer,
                key_package: &self.key_package,
                credential: &self.credential,
                epoch: self.epoch,
            },
        )?;
        let (transport, rng, clock) = inputs.take().ok_or(SyncError::JoinRejected)?;
        let graph = self.graph.take().expect("checked original recovered graph");
        let routing = RoutingState {
            label: 0,
            secrets: vec![(0, graph.routing_secret)],
            policy: Some(graph.policy),
            ..RoutingState::default()
        };
        let mut node =
            ChannelSync::new_joined(transport, graph.group, graph.device, rng, clock, routing);
        node.file_wrap_key = graph.file_key;
        ClosedV2JoinedNode::from_exact_recovered_graph(
            node,
            self.head,
            prefix.own_admission_data(),
            self.epoch,
            self.authenticator,
        )
    }
}
