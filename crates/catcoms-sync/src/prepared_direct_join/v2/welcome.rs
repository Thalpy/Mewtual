//! Verified Welcome processing on a disposable restore of the original
//! private provider. A completed candidate has no Legacy runtime conversion.
use super::*;
use crate::owner_v2::{V2JoinFacts, V2_JOIN_ROUTING_DOMAIN};
use catcoms_crypto::DeviceId;
use catcoms_mls::group::text_current_add::MAX_TEXT_ADD_ROUTING_PLAIN_BYTES;
use catcoms_replication::chat::{
    join_bindings::{SignedWelcomeBindingData, MAX_JOIN_PAYLOAD_BYTES},
    owner_facts::OwnerFactId,
    owner_header::{OwnerHeaderId, SignedOwnerHeader},
    owner_profile::{AdmissionBody, AdmissionOrigin, SignedOwnerProfile},
    owner_state::StateFrame,
    V2GeneralPrefixVerifier,
};

const MAX_ROUTING_FRAME: usize = MAX_TEXT_ADD_ROUTING_PLAIN_BYTES + 48;
const TAG: &[u8] = b"mewtual/chat/join-response-packet/v2\0";
const FRAME_OVERHEAD: usize = 4 + TAG.len() + 2 + 3 * 4;
pub const MAX_V2_JOIN_RESPONSE_PACKET_BYTES: usize =
    FRAME_OVERHEAD + MAX_JOIN_BINDING_BYTES + MAX_JOIN_PAYLOAD_BYTES + MAX_ROUTING_FRAME;

/// Exact signed response and its separately bounded payloads.
#[derive(Clone, Copy)]
pub struct V2JoinResponseData<'a> {
    pub binding: &'a SignedWelcomeBindingData,
    pub welcome: &'a [u8],
    pub sealed_routing: &'a [u8],
}

impl fmt::Debug for V2JoinResponseData<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("V2JoinResponseData").finish_non_exhaustive()
    }
}

impl V2JoinResponseData<'_> {
    /// Frame exact authenticated bytes. The owning host needs its separate
    /// covering and publication proof before releasing this frame.
    pub fn encode_packet_data(&self) -> Result<Vec<u8>, SyncError> {
        let (binding, length) = self.bounded_frame_data(MAX_V2_JOIN_RESPONSE_PACKET_BYTES)?;
        self.binding
            .check_payloads_data(self.welcome, self.sealed_routing)
            .map_err(|_| SyncError::JoinRejected)?;
        let malformed = |_| SyncError::Malformed;
        let mut e = Encoder::with_capacity(length);
        e.put_bytes(TAG).map_err(malformed)?;
        e.put_u16(VERSION);
        e.put_bytes(&binding).map_err(malformed)?;
        e.put_bytes(self.welcome).map_err(malformed)?;
        e.put_bytes(self.sealed_routing).map_err(malformed)?;
        Ok(e.finish())
    }

    fn bounded_frame_data(&self, cap: usize) -> Result<(Vec<u8>, usize), SyncError> {
        check_payload_bounds(self.welcome, self.sealed_routing)?;
        let binding = self.binding.encode();
        let length =
            FRAME_OVERHEAD + binding.len() + self.welcome.len() + self.sealed_routing.len();
        if binding.len() > MAX_JOIN_BINDING_BYTES || length > cap {
            return Err(SyncError::Malformed);
        }
        Ok((binding, length))
    }
}

/// Verified payload binding only. The joiner still checks the exact original
/// request and its independently verified Add before consuming the Welcome.
pub struct V2JoinResponsePacketData<'a> {
    binding: SignedWelcomeBindingData,
    welcome: &'a [u8],
    sealed_routing: &'a [u8],
}

impl<'a> V2JoinResponsePacketData<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, SyncError> {
        if bytes.len() > MAX_V2_JOIN_RESPONSE_PACKET_BYTES {
            return Err(SyncError::Malformed);
        }
        let malformed = |_| SyncError::Malformed;
        let mut d = Decoder::new(bytes);
        if d.get_bytes().map_err(malformed)? != TAG || d.get_u16().map_err(malformed)? != VERSION {
            return Err(SyncError::Malformed);
        }
        let binding = d.get_bytes().map_err(malformed)?;
        let welcome = d.get_bytes().map_err(malformed)?;
        let sealed_routing = d.get_bytes().map_err(malformed)?;
        d.finish().map_err(malformed)?;
        check_payload_bounds(welcome, sealed_routing)?;
        let binding =
            SignedWelcomeBindingData::decode(binding).map_err(|_| SyncError::Malformed)?;
        binding
            .check_payloads_data(welcome, sealed_routing)
            .map_err(|_| SyncError::JoinRejected)?;
        Ok(Self {
            binding,
            welcome,
            sealed_routing,
        })
    }

    pub fn response_data(&self) -> V2JoinResponseData<'_> {
        V2JoinResponseData {
            binding: &self.binding,
            welcome: self.welcome,
            sealed_routing: self.sealed_routing,
        }
    }
}

impl fmt::Debug for V2JoinResponsePacketData<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("V2JoinResponsePacketData")
            .finish_non_exhaustive()
    }
}

fn check_payload_bounds(welcome: &[u8], sealed_routing: &[u8]) -> Result<(), SyncError> {
    if welcome.is_empty()
        || welcome.len() > MAX_JOIN_PAYLOAD_BYTES
        || sealed_routing.is_empty()
        || sealed_routing.len() > MAX_ROUTING_FRAME
    {
        return Err(SyncError::Malformed);
    }
    Ok(())
}

/// An actual private selected-KP Welcome result, kept with the original
/// request. Durable joined pin, scope, source, and installation are separate.
pub struct CompletedV2JoinCandidate {
    pub(super) device: MlsDevice,
    pub(super) group: ServerGroup,
    pub(super) facts: V2JoinFacts,
    pub(super) policy: GroupPolicy,
    pub(super) routing_secret: Zeroizing<[u8; 32]>,
    pub(super) file_key: Zeroizing<[u8; 32]>,
    pub(super) response_binding: Vec<u8>,
    pub(super) state: catcoms_replication::chat::owner_header::OwnerStateCommitment,
}

impl CompletedV2JoinCandidate {
    pub fn facts_data(&self) -> &V2JoinFacts {
        &self.facts
    }

    pub fn policy_data(&self) -> &GroupPolicy {
        &self.policy
    }

    /// A negative check on the retained actual candidate, never an adoption API.
    pub fn validate_initial_state_data(&self) -> Result<(), SyncError> {
        if !self.facts.matches_initial_state(&self.group, &self.device) {
            return Err(SyncError::JoinRejected);
        }
        Ok(())
    }
}

impl fmt::Debug for CompletedV2JoinCandidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompletedV2JoinCandidate")
            .finish_non_exhaustive()
    }
}

pub(super) struct WelcomeAdmission<'a> {
    profile: &'a SignedOwnerProfile,
    body: &'a AdmissionBody,
    pub(super) header: &'a SignedOwnerHeader,
    state_image: &'a [u8],
    epoch: u64,
}

pub(super) struct ValidatedWelcome<'a> {
    pub(super) binding: Vec<u8>,
    pub(super) state: StateFrame<'a>,
    pub(super) before_epoch: u64,
    pub(super) after_epoch: u64,
}

impl<'a> WelcomeAdmission<'a> {
    fn first(
        proof: &'a V2FirstAddProofData<'_>,
        anchor: &GenesisAnchorBindingData,
    ) -> Result<Self, SyncError> {
        let (profile, body, header) = proof.verified_admission(anchor)?;
        Ok(Self {
            profile,
            body,
            header,
            state_image: proof.add_image_data(),
            epoch: 1,
        })
    }

    fn later(
        proof: &'a V2GoverningJoinPrefixData<'_>,
        anchor: &GenesisAnchorBindingData,
        signed: &SignedWelcomeBindingData,
    ) -> Result<Self, SyncError> {
        let labels = signed.spec_data();
        let replay = proof.verify_selected_add(
            anchor,
            OwnerHeaderId::from_bytes(labels.add_header),
            OwnerFactId::from_bytes(labels.admission),
        )?;
        let (last, body) = proof.last_admission()?;
        Ok(Self {
            profile: proof.profile_data(),
            body,
            header: last.header(),
            state_image: last.image(),
            epoch: replay.mls_epoch_data(),
        })
    }

    pub(super) fn for_delivery(
        delivery: &'a V2JoinDeliveryData<'_>,
        anchor: &GenesisAnchorBindingData,
    ) -> Result<Self, SyncError> {
        match delivery {
            V2JoinDeliveryData::First(frame) => Self::first(frame.data().1, anchor),
            V2JoinDeliveryData::Later(frame) => Self::later(
                frame.governing_proof_data().0,
                anchor,
                frame.response_data().binding,
            ),
        }
    }

    pub(super) fn validate(
        &self,
        packet: &V2JoinRequestPacketData<'_>,
        response: V2JoinResponseData<'_>,
        cap: usize,
    ) -> Result<ValidatedWelcome<'a>, SyncError> {
        let (binding, _) = response.bounded_frame_data(cap)?;
        let reject = |_| SyncError::JoinRejected;
        let AdmissionOrigin::Add {
            before_epoch,
            after_epoch,
            ..
        } = self.body.spec().origin
        else {
            return Err(SyncError::JoinRejected);
        };
        if after_epoch != self.epoch || before_epoch.checked_add(1) != Some(after_epoch) {
            return Err(SyncError::JoinRejected);
        }
        response
            .binding
            .check_links_data(
                &packet.invite,
                &packet.request,
                self.profile,
                self.body,
                self.header,
            )
            .map_err(reject)?;
        response
            .binding
            .check_payloads_data(response.welcome, response.sealed_routing)
            .map_err(reject)?;
        let state = StateFrame::decode(self.state_image).map_err(reject)?;
        Ok(ValidatedWelcome {
            binding,
            state,
            before_epoch,
            after_epoch,
        })
    }
}

impl PreparedV2JoinRequest {
    /// Fund retention of the original warm private graph before Welcome entry.
    /// A failed or completed entry cannot be relabelled with a different fee.
    pub fn fund_warm_sdk_graph(
        &mut self,
        payment: catcoms_mls::persist::TextCurrentSdkRestoreAdmission,
    ) -> Result<(), SyncError> {
        if self.welcome_entered || self.warm_sdk_payment.is_some() {
            return Err(SyncError::JoinRejected);
        }
        self.warm_sdk_payment = Some(payment);
        Ok(())
    }

    /// The verifier must replay the accepted genesis through this exact Add.
    /// Authenticate the response and prefix *before* restoring or entering the
    /// Welcome SDK. Failed candidates leave the original private bundle intact.
    /// Exact already-completed responses reuse the same private result.
    pub fn finish_welcome(
        &mut self,
        response: V2JoinResponseData<'_>,
        prefix: &V2GeneralPrefixVerifier,
    ) -> Result<&CompletedV2JoinCandidate, SyncError> {
        let reject = |_| SyncError::JoinRejected;
        let request_bytes = self.request.clone();
        let packet = V2JoinRequestPacketData::decode(&request_bytes)?;
        let anchor = packet.request.spec_data().anchor;
        prefix.require_join_genesis_data(anchor).map_err(reject)?;
        let replay = prefix.replay_data();
        let admission = prefix
            .admission(OwnerFactId::from_bytes(
                response.binding.spec_data().admission,
            ))
            .map_err(reject)?;
        if !admission.is_open_data() {
            return Err(SyncError::JoinRejected);
        }
        let cap = self.limits.exchange;
        let result = self.finish_verified_welcome(
            &packet,
            response,
            WelcomeAdmission {
                profile: replay.profile(),
                body: admission.admission_data(),
                header: replay.header(),
                state_image: replay.image(),
                epoch: replay.mls_epoch_data(),
            },
            cap,
            WelcomeAuthority::default(),
        )?;
        prefix.require_join_genesis_data(anchor).map_err(reject)?;
        if !admission.is_open_data() {
            return Err(SyncError::JoinRejected);
        }
        Ok(result)
    }

    /// Replay this request's accepted genesis and the first governing Add
    /// from the supplied public records, then process its exact Welcome.
    /// The resulting private candidate still needs its original joined pin
    /// and native installation before it can author or publish.
    pub fn finish_first_welcome(
        &mut self,
        response: V2JoinResponseData<'_>,
        proof: &V2FirstAddProofData<'_>,
    ) -> Result<&CompletedV2JoinCandidate, SyncError> {
        self.finish_first_welcome_with(response, proof, WelcomeAuthority::default())
    }

    /// Verify the complete prefix through the exact Add named by this Welcome,
    /// then process the same private MLS candidate and check its resulting state.
    pub fn finish_governing_welcome(
        &mut self,
        response: V2JoinResponseData<'_>,
        proof: &V2GoverningJoinPrefixData<'_>,
    ) -> Result<&CompletedV2JoinCandidate, SyncError> {
        let cap = self
            .limits
            .exchange
            .checked_sub(proof.frame_bytes())
            .ok_or(SyncError::Malformed)?;
        let request_bytes = self.request.clone();
        let packet = V2JoinRequestPacketData::decode(&request_bytes)?;
        let admission =
            WelcomeAdmission::later(proof, &packet.invite.spec_data().anchor, response.binding)?;
        self.finish_verified_welcome(
            &packet,
            response,
            admission,
            cap,
            WelcomeAuthority::default(),
        )
    }

    /// Finish a later Welcome from its bounded connected response. Retain the
    /// original private request and separately verify the entire public proof.
    pub fn finish_governing_join_delivery(
        &mut self,
        delivery: &V2GoverningJoinDeliveryData<'_>,
    ) -> Result<&CompletedV2JoinCandidate, SyncError> {
        if delivery.frame_bytes() > self.limits.exchange {
            return Err(SyncError::Malformed);
        }
        self.finish_governing_welcome(delivery.response_data(), delivery.governing_proof_data().0)
    }

    fn finish_first_welcome_with(
        &mut self,
        response: V2JoinResponseData<'_>,
        proof: &V2FirstAddProofData<'_>,
        authority: WelcomeAuthority<'_>,
    ) -> Result<&CompletedV2JoinCandidate, SyncError> {
        let cap = self
            .limits
            .exchange
            .checked_sub(proof.frame_bytes())
            .ok_or(SyncError::Malformed)?;
        let request_bytes = self.request.clone();
        let packet = V2JoinRequestPacketData::decode(&request_bytes)?;
        let admission = WelcomeAdmission::first(proof, &packet.invite.spec_data().anchor)?;
        self.finish_verified_welcome(&packet, response, admission, cap, authority)
    }

    /// Verify the full delivery without restoring the private provider or
    /// consuming the original Welcome attempt. The mounted owner can cover its
    /// exact possible-entry record after this check and before calling finish.
    pub fn validate_first_join_delivery_data(&self, bytes: &[u8]) -> Result<(), SyncError> {
        validate_delivery_data(&self.request, self.limits, bytes)
    }

    /// Authenticate a complete first or later delivery before the host covers
    /// this exact possible Welcome entry.
    pub fn validate_pinned_join_delivery_data(&self, bytes: &[u8]) -> Result<(), SyncError> {
        validate_pinned_delivery_data(&self.request, self.limits, bytes)
    }

    /// Enter the exact covered Welcome on the original request using the
    /// host's original cancellation and native key cut when present.
    pub fn finish_pinned_join_delivery(
        &mut self,
        bytes: &[u8],
        negative: Option<&catcoms_rt::RequestCancellation>,
        native: Option<NativeV2JoinKeyEntry<'_>>,
    ) -> Result<&CompletedV2JoinCandidate, SyncError> {
        if bytes.len() > self.limits.exchange || (native.is_some() && negative.is_none()) {
            return Err(SyncError::JoinRejected);
        }
        let delivery = V2JoinDeliveryData::decode(bytes)?;
        let request_bytes = self.request.clone();
        let packet = V2JoinRequestPacketData::decode(&request_bytes)?;
        let admission =
            WelcomeAdmission::for_delivery(&delivery, &packet.invite.spec_data().anchor)?;
        self.finish_verified_welcome(
            &packet,
            delivery.response_data(),
            admission,
            self.limits.exchange,
            WelcomeAuthority { negative, native },
        )
    }

    /// Charge the complete wire response, including the history and framing,
    /// to the original exchange limit before verifying or entering the SDK.
    pub fn finish_first_join_delivery(
        &mut self,
        bytes: &[u8],
    ) -> Result<&CompletedV2JoinCandidate, SyncError> {
        self.finish_first_join_delivery_with(bytes, WelcomeAuthority::default())
    }

    /// The owning host retains its original cancellation until the exact
    /// Welcome would first enter the SDK. Revocation at that edge leaves the
    /// same private package available for an authorized exact retry.
    pub fn finish_first_join_delivery_cancellable(
        &mut self,
        bytes: &[u8],
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<&CompletedV2JoinCandidate, SyncError> {
        self.finish_first_join_delivery_with(
            bytes,
            WelcomeAuthority {
                negative: Some(negative),
                native: None,
            },
        )
    }

    /// Enter the actual native joiner's private MLS work under its original
    /// source and current UI, after authenticating the covered delivery.
    pub fn finish_native_first_join_delivery(
        &mut self,
        bytes: &[u8],
        negative: &catcoms_rt::RequestCancellation,
        key: NativeV2JoinKeyEntry<'_>,
    ) -> Result<&CompletedV2JoinCandidate, SyncError> {
        self.finish_first_join_delivery_with(
            bytes,
            WelcomeAuthority {
                negative: Some(negative),
                native: Some(key),
            },
        )
    }

    fn finish_first_join_delivery_with(
        &mut self,
        bytes: &[u8],
        authority: WelcomeAuthority<'_>,
    ) -> Result<&CompletedV2JoinCandidate, SyncError> {
        if bytes.len() > self.limits.exchange {
            return Err(SyncError::Malformed);
        }
        let delivery = V2FirstJoinDeliveryData::decode(bytes)?;
        let (response, proof) = delivery.data();
        self.finish_first_welcome_with(response, proof, authority)
    }

    fn finish_verified_welcome(
        &mut self,
        packet: &V2JoinRequestPacketData<'_>,
        response: V2JoinResponseData<'_>,
        admission: WelcomeAdmission<'_>,
        cap: usize,
        mut authority: WelcomeAuthority<'_>,
    ) -> Result<&CompletedV2JoinCandidate, SyncError> {
        let ValidatedWelcome {
            binding: response_binding,
            state,
            before_epoch,
            after_epoch,
        } = admission.validate(packet, response, cap)?;
        let header = admission.header;
        let request = packet.request.spec_data();
        if authority
            .negative
            .is_some_and(catcoms_rt::RequestCancellation::is_cancelled)
        {
            return Err(SyncError::JoinRejected);
        }
        match (&self.native_origin, &authority.native) {
            (None, None) => (),
            (Some(original), Some(key)) if original.same_original(key.original) => {
                key.cut
                    .preflight(original, NativeChatKeyPurpose::JoinerWelcomeSdk)
                    .map_err(|_| SyncError::JoinRejected)?;
            }
            _ => return Err(SyncError::JoinRejected),
        }
        if self.completed.is_none() {
            if self.welcome_entered {
                return Err(SyncError::JoinRejected);
            }
            if let Some(key) = authority.native.as_mut() {
                key.enter(NativeChatKeyPurpose::JoinerWelcomeSdk)?;
            }
            // Only a fully authenticated response can spend the original
            // finite SDK attempt. Ambiguous or failed SDK entry stays held.
            self.welcome_entered = true;
            let mut device = self.restore_exact_device(packet)?;
            let group = if self.warm_sdk_payment.is_some() {
                ServerGroup::join_closed_for_v2_key_package(
                    &mut device,
                    response.welcome,
                    &request.key_package_reference,
                    &mut self.warm_sdk_payment,
                )?
            } else {
                ServerGroup::join_for_v2_key_package(
                    &device,
                    response.welcome,
                    &request.key_package_reference,
                )?
            };
            let facts = V2JoinFacts::capture(&group, &device, request.key_package_reference)?;
            if facts.group_data() != packet.invite.spec_data().anchor.group.as_slice()
                || facts.signer_data() != &request.joiner
                || facts.credential_data() != &request.credential
                || facts.join_epoch_data() != after_epoch
                || !matches_welcome_roster(&group, &state)
            {
                return Err(SyncError::JoinRejected);
            }
            let key = Zeroizing::new(group.routing_transfer_key(&device)?);
            let plain = Zeroizing::new(
                unseal(&key, &decode_sealed(response.sealed_routing)?)
                    .map_err(|_| SyncError::JoinRejected)?,
            );
            let (policy, routing_secret, file_key) = open_routing(
                &plain,
                packet,
                before_epoch,
                after_epoch,
                header.spec().parent.as_bytes(),
            )?;
            policy.verify_current_owner(&group)?;
            // Welcome grows the actual provider. Verify its resulting backing
            // and retained encoding within the original caller's allowance.
            drop(snapshot_join_device_bounded(
                &device,
                self.limits.provider,
                self.limits.backing,
            )?);
            // Record the verified candidate before any late prefix or
            // admission revocation can suppress the response.
            self.completed = Some(Box::new(CompletedV2JoinCandidate {
                device,
                group,
                facts,
                policy,
                routing_secret,
                file_key,
                response_binding,
                state: header.spec().state,
            }));
        } else if self
            .completed
            .as_ref()
            .is_some_and(|completed| completed.response_binding != response_binding)
        {
            return Err(SyncError::JoinRejected);
        }
        let result = self.completed.as_ref().ok_or(SyncError::JoinRejected)?;
        result.validate_initial_state_data()?;
        Ok(result)
    }
}

#[derive(Default)]
struct WelcomeAuthority<'a> {
    negative: Option<&'a catcoms_rt::RequestCancellation>,
    native: Option<NativeV2JoinKeyEntry<'a>>,
}

pub(super) fn validate_delivery_data(
    request: &[u8],
    limits: DirectJoinLimits,
    bytes: &[u8],
) -> Result<(), SyncError> {
    if bytes.len() > limits.exchange {
        return Err(SyncError::Malformed);
    }
    let delivery = V2FirstJoinDeliveryData::decode(bytes)?;
    let (response, proof) = delivery.data();
    let cap = limits
        .exchange
        .checked_sub(proof.frame_bytes())
        .ok_or(SyncError::Malformed)?;
    let packet = V2JoinRequestPacketData::decode(request)?;
    let admission = WelcomeAdmission::first(proof, &packet.invite.spec_data().anchor)?;
    admission.validate(&packet, response, cap)?;
    Ok(())
}

pub(super) fn validate_pinned_delivery_data(
    request: &[u8],
    limits: DirectJoinLimits,
    bytes: &[u8],
) -> Result<(), SyncError> {
    if bytes.len() > limits.exchange {
        return Err(SyncError::Malformed);
    }
    let delivery = V2JoinDeliveryData::decode(bytes)?;
    let packet = V2JoinRequestPacketData::decode(request)?;
    WelcomeAdmission::for_delivery(&delivery, &packet.invite.spec_data().anchor)?.validate(
        &packet,
        delivery.response_data(),
        limits.exchange,
    )?;
    Ok(())
}

pub(super) fn matches_welcome_roster(group: &ServerGroup, state: &StateFrame) -> bool {
    group.member_count() == state.slots().len()
        && state.slots().all(|slot| {
            group
                .member_signature_key(&DeviceId::from_public_key_bytes(&slot.subject))
                .is_some_and(|key| key == slot.subject)
        })
}

type RoutingTransfer = (GroupPolicy, Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>);

pub(super) fn open_routing(
    plain: &[u8],
    packet: &V2JoinRequestPacketData<'_>,
    before_epoch: u64,
    after_epoch: u64,
    parent: &[u8; 32],
) -> Result<RoutingTransfer, SyncError> {
    if plain.len() > MAX_TEXT_ADD_ROUTING_PLAIN_BYTES {
        return Err(SyncError::Malformed);
    }
    let malformed = |_| SyncError::Malformed;
    let request = packet.request.spec_data();
    let invite = packet.invite.spec_data();
    let mut d = Decoder::new(plain);
    if d.get_bytes().map_err(malformed)? != V2_JOIN_ROUTING_DOMAIN
        || d.get_u16().map_err(malformed)? != 2
    {
        return Err(SyncError::Malformed);
    }
    let request_id = packet.request.id_data();
    for expected in [
        invite.anchor.group.as_slice(),
        request.anchor.as_slice(),
        request_id.as_slice(),
        request.key_package_wire.as_slice(),
        request.joiner.as_slice(),
        request.key_package_reference.as_slice(),
        request.credential.as_slice(),
    ] {
        if d.get_bytes().map_err(malformed)? != expected {
            return Err(SyncError::JoinRejected);
        }
    }
    if d.get_u64().map_err(malformed)? != before_epoch
        || d.get_u64().map_err(malformed)? != after_epoch
        || d.get_bytes().map_err(malformed)? != parent
        || d.get_bytes().map_err(malformed)? != invite.policy
        || d.get_bytes().map_err(malformed)? != invite.public_routing_plan
    {
        return Err(SyncError::JoinRejected);
    }
    let policy_bytes = d.get_bytes().map_err(malformed)?;
    if policy_bytes.len() > 4096
        || JoinPayloadKindData::Policy
            .commitment_data(policy_bytes)
            .map_err(|_| SyncError::JoinRejected)?
            != invite.policy
    {
        return Err(SyncError::JoinRejected);
    }
    if d.get_u64().map_err(malformed)? != 0
        || d.get_u32().map_err(malformed)? != 1
        || d.get_u64().map_err(malformed)? != 0
    {
        return Err(SyncError::JoinRejected);
    }
    let routing = Zeroizing::new(
        d.get_bytes()
            .map_err(malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?,
    );
    let file_key = Zeroizing::new(
        d.get_bytes()
            .map_err(malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?,
    );
    d.finish().map_err(malformed)?;
    let policy = GroupPolicy::decode(policy_bytes)?;
    crate::group_policy::require_supported(&policy)?;
    Ok((policy, routing, file_key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_crypto::DeviceKeypair;
    use catcoms_replication::chat::join_bindings::WelcomeBindingSpecData;

    #[test]
    fn join_response_budget_counts_the_exact_frame() {
        let founder = DeviceKeypair::from_seed(&[3; 32]);
        let welcome = b"welcome";
        let sealed_routing = b"routing";
        let binding = SignedWelcomeBindingData::sign_using(
            WelcomeBindingSpecData {
                invite: [1; 32],
                anchor: [2; 32],
                request_binding: [3; 32],
                key_package_wire: [4; 32],
                key_package_reference: [5; 32],
                admitter: founder.verifying_key().to_bytes(),
                add_header: [6; 32],
                admission: [7; 32],
                welcome: JoinPayloadKindData::Welcome
                    .commitment_data(welcome)
                    .unwrap(),
                policy: [8; 32],
                public_routing_plan: [9; 32],
                sealed_routing_transfer: JoinPayloadKindData::RoutingTransfer
                    .commitment_data(sealed_routing)
                    .unwrap(),
            },
            |bytes| Ok(founder.sign(bytes)),
        )
        .unwrap();
        let response = V2JoinResponseData {
            binding: &binding,
            welcome,
            sealed_routing,
        };
        let wire = response.encode_packet_data().unwrap();
        assert_eq!(
            response.bounded_frame_data(wire.len()).unwrap().1,
            wire.len()
        );
        assert!(response.bounded_frame_data(wire.len() - 1).is_err());
    }
}
