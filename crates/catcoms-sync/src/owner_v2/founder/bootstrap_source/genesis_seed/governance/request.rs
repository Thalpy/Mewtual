//! Frozen signed request and original routing custody. These checks establish
//! cryptographic consistency with THIS node, not Root invitation permission,
//! nonce consumption, a saved operation or a current selected governing head.
use super::super::super::codec::VerifiedGenesisData;
use super::*;
use catcoms_mls::group::text_current_add::MAX_TEXT_ADD_ROUTING_PLAIN_BYTES;
use catcoms_replication::chat::join_bindings::{
    JoinPayloadKindData, JoinRequestBindingSpecData, SignedInviteBindingData,
    SignedJoinRequestBindingData,
};

/// Complete DATA input, with no renderer time or separately supplied nonce.
/// Each representation is authenticated before comparison with a retained
/// request. Root still authorizes the inviter, public route plan and policy.
#[derive(Clone, Copy)]
pub struct V2FounderAddRequestData<'a> {
    pub invite: &'a SignedInviteBindingData,
    pub request: &'a SignedJoinRequestBindingData,
    pub key_package: &'a [u8],
}
impl std::fmt::Debug for V2FounderAddRequestData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2FounderAddRequestData")
            .finish_non_exhaustive()
    }
}

/// Bound BEFORE hashing/copying. The SDK independently applies its own TLS,
/// credential, reference, group/nonce and actual admission-time lifetime checks.
pub const MAX_FOUNDER_ADD_KEY_PACKAGE_BYTES: usize = 32 * 1024;

pub(super) struct FrozenAddRequest {
    invite: Vec<u8>,
    request: Vec<u8>,
    key_package: Vec<u8>,
    pub(super) spec: JoinRequestBindingSpecData,
    pub(super) request_id: [u8; 32],
    pub(super) policy: [u8; 32],
    pub(super) public_routing_plan: [u8; 32],
    nonce: [u8; 16],
    expires_at_ms: u64,
    pub(super) before_epoch: u64,
    pub(super) after_epoch: u64,
    pub(super) parent: OwnerHeaderId,
    // Secret source and AEAD scratch never escape via DATA/debug getters.
    pub(super) routing_plain: Zeroizing<Vec<u8>>,
}

impl V2FounderAddRequestData<'_> {
    fn verify(self) -> Result<(), SyncError> {
        if self.key_package.is_empty() || self.key_package.len() > MAX_FOUNDER_ADD_KEY_PACKAGE_BYTES
        {
            return Err(unavailable());
        }
        // Verify BOTH signatures on EVERY representation, even an invalid
        // signature sharing a previously accepted semantic ID.
        self.request
            .check_invite_links_data(self.invite)
            .map_err(|_| unavailable())?;
        self.request
            .check_key_package_wire_data(self.key_package)
            .map_err(|_| unavailable())
    }

    pub(super) fn verify_seed(self, seed: &SeedBody) -> Result<Vec<u8>, SyncError> {
        if seed.metadata.routing_label != 0
            || seed.metadata.routing.len() != 1
            || seed.metadata.routing[0].0 != 0
        {
            return Err(unavailable());
        }
        self.verify_original(
            &seed.context,
            &seed.genesis,
            seed.metadata.policy.as_ref().ok_or_else(unavailable)?,
        )
    }

    fn verify_original(
        self,
        context: &BootstrapSourceContextData,
        genesis: &VerifiedGenesisData,
        policy: &GroupPolicy,
    ) -> Result<Vec<u8>, SyncError> {
        self.verify()?;
        let invite = self.invite.spec_data();
        let anchor = &invite.anchor;
        if anchor.group != genesis.profile.spec().group
            || anchor.founder != genesis.profile.spec().founder
            || anchor.profile != *genesis.profile.id().as_bytes()
            || anchor.admission != *genesis.admission.admission_id().as_bytes()
            || anchor.tenure != *genesis.profile.tenure().as_bytes()
            || anchor.header != *genesis.header.id().as_bytes()
            || anchor.state != *genesis.header.spec().state.as_bytes()
            || invite.inviter != context.spec_data().signer
        {
            return Err(unavailable());
        }
        crate::group_policy::require_supported(policy).map_err(|_| unavailable())?;
        let policy = policy.encode();
        if JoinPayloadKindData::Policy
            .commitment_data(&policy)
            .map_err(|_| unavailable())?
            != invite.policy
        {
            return Err(unavailable());
        }
        Ok(policy)
    }

    /// Match a recovered founder request to the original encrypted S0
    /// metadata. Never use the successor MLS exporter for the shared L0 keys.
    pub(super) fn verify_recovered<'source>(
        self,
        original: &'source OpenedBootstrapSourceData,
    ) -> Result<(Vec<u8>, &'source [u8; 32], &'source [u8; 32]), SyncError> {
        let source = original.view_data();
        let fields = s0::fields(source.snapshot, source.context.spec_data())?;
        let mut route = catcoms_wire::Decoder::new(fields.routing);
        let invalid = |_| unavailable();
        if route.get_u64().map_err(invalid)? != 0
            || route.get_u32().map_err(invalid)? != 1
            || route.get_u64().map_err(invalid)? != 0
        {
            return Err(unavailable());
        }
        let l0 = route
            .get_bytes()
            .map_err(invalid)?
            .try_into()
            .map_err(|_| unavailable())?;
        route.finish().map_err(invalid)?;
        let policy = crate::group_policy::decode_pin(fields.policy)?.ok_or_else(unavailable)?;
        let policy = self.verify_original(source.context, &original.genesis, &policy)?;
        Ok((policy, l0, fields.file_wrap_key))
    }

    pub(super) fn check_live_package(
        self,
        (device, group): (&MlsDevice, &ServerGroup),
        now: u64,
        limits: catcoms_mls::persist::text_current_add::TextCurrentAddLimits,
    ) -> Result<(), SyncError> {
        if self.invite.expired_at_data(now) {
            return Err(unavailable());
        }
        let signed = self.request.spec_data();
        let packet = TextCurrentAddRequestData::from_untrusted_data(
            self.key_package,
            signed.joiner,
            self.invite.spec_data().nonce,
            signed.key_package_reference,
            now,
        )
        .with_expected_credential_data(signed.credential);
        catcoms_mls::group::text_current_add::check_private_text_add_package(
            device, group, packet, limits,
        )
        .map_err(|_| unavailable())
    }
}

impl FrozenAddRequest {
    pub(super) fn invite_nonce(&self) -> [u8; 16] {
        self.nonce
    }

    pub(super) fn prepare(
        seed: &SeedBody,
        installed: Option<V2InstalledFounderAddData>,
        data: V2FounderAddRequestData<'_>,
    ) -> Result<Self, SyncError> {
        Self::freeze(
            data,
            data.verify_seed(seed)?,
            (&seed.metadata.routing[0].1, &seed.metadata._file_wrap_key),
            (
                seed.group.epoch(),
                installed.map_or(seed.genesis.header.id(), |selected| selected.header),
            ),
        )
    }

    pub(super) fn prepare_recovered(
        source: &OpenedBootstrapSourceData,
        installed: V2InstalledFounderAddData,
        data: V2FounderAddRequestData<'_>,
    ) -> Result<Self, SyncError> {
        let (policy, l0, file_key) = data.verify_recovered(source)?;
        Self::freeze(
            data,
            policy,
            (l0, file_key),
            (installed.epoch, installed.header),
        )
    }

    fn freeze(
        data: V2FounderAddRequestData<'_>,
        policy: Vec<u8>,
        (l0, file_key): (&[u8; 32], &[u8; 32]),
        (before_epoch, parent): (u64, OwnerHeaderId),
    ) -> Result<Self, SyncError> {
        let invite = data.invite.spec_data();
        // The wrapper admits Add only. No Remove or policy mutation can make
        // these original L0 secrets stale. Future transition support MUST
        // install genuine updated routing custody, never rederive L0 from a
        // post-Add exporter (only the transfer wrapping key changes on Add).
        let spec = data.request.spec_data().clone();
        let after_epoch = before_epoch.checked_add(1).ok_or_else(unavailable)?;
        // Explicit V2 framing; none of this is the legacy routing grammar.
        // Preallocate the full supported cap INSIDE Zeroizing before the first
        // secret copy, so an early return/unwind also clears the partial frame.
        let mut plain = Zeroizing::new(Vec::with_capacity(MAX_TEXT_ADD_ROUTING_PLAIN_BYTES));
        field(&mut plain, crate::owner_v2::V2_JOIN_ROUTING_DOMAIN)?;
        plain.extend_from_slice(&2u16.to_be_bytes());
        field(&mut plain, &invite.anchor.group)?;
        field(&mut plain, &spec.anchor)?;
        field(&mut plain, &data.request.id_data())?;
        field(&mut plain, &spec.key_package_wire)?;
        field(&mut plain, &spec.joiner)?;
        field(&mut plain, &spec.key_package_reference)?;
        field(&mut plain, &spec.credential)?;
        plain.extend_from_slice(&before_epoch.to_be_bytes());
        plain.extend_from_slice(&after_epoch.to_be_bytes());
        field(&mut plain, parent.as_bytes())?;
        field(&mut plain, &invite.policy)?;
        field(&mut plain, &invite.public_routing_plan)?;
        field(&mut plain, &policy)?;
        plain.extend_from_slice(&0u64.to_be_bytes()); // current label L0
        plain.extend_from_slice(&1u32.to_be_bytes()); // exactly one slot
        plain.extend_from_slice(&0u64.to_be_bytes()); // slot L0
        field(&mut plain, l0)?;
        field(&mut plain, file_key)?;
        Ok(Self {
            invite: data.invite.encode(),
            request: data.request.encode(),
            key_package: data.key_package.to_vec(),
            spec,
            request_id: data.request.id_data(),
            policy: invite.policy,
            public_routing_plan: invite.public_routing_plan,
            nonce: invite.nonce,
            expires_at_ms: invite.expires_at_ms,
            before_epoch,
            after_epoch,
            parent,
            routing_plain: plain,
        })
    }

    pub(super) fn require_same(&self, data: V2FounderAddRequestData<'_>) -> Result<(), SyncError> {
        data.verify()?;
        if data.key_package != self.key_package
            || data.invite.encode() != self.invite
            || data.request.encode() != self.request
        {
            return Err(unavailable());
        }
        Ok(())
    }

    pub(super) fn require_unexpired(&self, now_ms: u64) -> Result<(), SyncError> {
        if now_ms >= self.expires_at_ms {
            Err(unavailable())
        } else {
            Ok(())
        }
    }

    pub(super) fn sdk_request(&self, admitted_at_ms: u64) -> TextCurrentAddRequestData<'_> {
        TextCurrentAddRequestData::from_untrusted_data(
            &self.key_package,
            self.spec.joiner,
            self.nonce,
            self.spec.key_package_reference,
            admitted_at_ms,
        )
        .with_expected_credential_data(self.spec.credential)
    }
}

fn field(out: &mut Zeroizing<Vec<u8>>, value: &[u8]) -> Result<(), SyncError> {
    let next = out
        .len()
        .checked_add(4)
        .and_then(|n| n.checked_add(value.len()));
    if next.is_none_or(|n| n > MAX_TEXT_ADD_ROUTING_PLAIN_BYTES) {
        return Err(unavailable());
    }
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value);
    Ok(())
}
