//! Verified Welcome processing on a disposable restore of the original
//! private provider. A completed candidate has no Legacy runtime conversion.
use super::*;
use crate::owner_v2::{V2JoinFacts, V2_JOIN_ROUTING_DOMAIN};
use catcoms_mls::group::text_current_add::MAX_TEXT_ADD_ROUTING_PLAIN_BYTES;
use catcoms_replication::chat::{
    join_bindings::{SignedWelcomeBindingData, MAX_JOIN_PAYLOAD_BYTES},
    owner_facts::OwnerFactId,
    owner_profile::AdmissionOrigin,
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
    device: MlsDevice,
    group: ServerGroup,
    facts: V2JoinFacts,
    policy: GroupPolicy,
    _routing_secret: Zeroizing<[u8; 32]>,
    _file_key: Zeroizing<[u8; 32]>,
    response_binding: Vec<u8>,
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

impl PreparedV2JoinRequest {
    /// The verifier must replay the accepted genesis through this exact Add.
    /// Authenticate the response and prefix *before* restoring or entering the
    /// Welcome SDK. Failed candidates leave the original private bundle intact.
    /// Exact already-completed responses reuse the same private result.
    pub fn finish_welcome(
        &mut self,
        response: V2JoinResponseData<'_>,
        prefix: &V2GeneralPrefixVerifier,
    ) -> Result<&CompletedV2JoinCandidate, SyncError> {
        let (response_binding, _) = response.bounded_frame_data(self.limits.exchange)?;
        let reject = |_| SyncError::JoinRejected;
        let packet = V2JoinRequestPacketData::decode(&self.request)?;
        let request = packet.request.spec_data();
        prefix
            .require_join_genesis_data(request.anchor)
            .map_err(reject)?;
        let replay = prefix.replay_data();
        let header = replay.header();
        let admission = prefix
            .admission(OwnerFactId::from_bytes(
                response.binding.spec_data().admission,
            ))
            .map_err(reject)?;
        if !admission.is_open_data() {
            return Err(SyncError::JoinRejected);
        }
        let body = admission.admission_data();
        let AdmissionOrigin::Add {
            before_epoch,
            after_epoch,
            ..
        } = body.spec().origin
        else {
            return Err(SyncError::JoinRejected);
        };
        if after_epoch != replay.mls_epoch_data()
            || before_epoch.checked_add(1) != Some(after_epoch)
        {
            return Err(SyncError::JoinRejected);
        }
        response
            .binding
            .check_links_data(
                &packet.invite,
                &packet.request,
                replay.profile(),
                body,
                header,
            )
            .map_err(reject)?;
        response
            .binding
            .check_payloads_data(response.welcome, response.sealed_routing)
            .map_err(reject)?;
        if self.completed.is_none() {
            if self.welcome_entered {
                return Err(SyncError::JoinRejected);
            }
            // Only a fully authenticated response can spend the original
            // finite SDK attempt. Ambiguous or failed SDK entry stays held.
            self.welcome_entered = true;
            let device = self.restore_exact_device(&packet)?;
            let group = ServerGroup::join_for_v2_key_package(
                &device,
                response.welcome,
                &request.key_package_reference,
            )?;
            let facts = V2JoinFacts::capture(&group, &device, request.key_package_reference)?;
            if facts.group_data() != packet.invite.spec_data().anchor.group.as_slice()
                || facts.signer_data() != &request.joiner
                || facts.credential_data() != &request.credential
                || facts.join_epoch_data() != after_epoch
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
                &packet,
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
            self.completed = Some(CompletedV2JoinCandidate {
                device,
                group,
                facts,
                policy,
                _routing_secret: routing_secret,
                _file_key: file_key,
                response_binding,
            });
        } else if self
            .completed
            .as_ref()
            .is_some_and(|completed| completed.response_binding != response_binding)
        {
            return Err(SyncError::JoinRejected);
        }
        let result = self.completed.as_ref().ok_or(SyncError::JoinRejected)?;
        result.validate_initial_state_data()?;
        prefix
            .require_join_genesis_data(request.anchor)
            .map_err(reject)?;
        if !admission.is_open_data() {
            return Err(SyncError::JoinRejected);
        }
        Ok(result)
    }
}

type RoutingTransfer = (GroupPolicy, Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>);

fn open_routing(
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
