//! Frozen signed request and original routing custody. These checks establish
//! cryptographic consistency with THIS node, not Root invitation permission,
//! nonce consumption, a saved operation or a current selected governing head.
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
        self.verify()?;
        let g = &seed.genesis;
        let invite = self.invite.spec_data();
        let anchor = &invite.anchor;
        if anchor.group != g.profile.spec().group
            || anchor.founder != g.profile.spec().founder
            || anchor.profile != *g.profile.id().as_bytes()
            || anchor.admission != *g.admission.admission_id().as_bytes()
            || anchor.tenure != *g.profile.tenure().as_bytes()
            || anchor.header != *g.header.id().as_bytes()
            || anchor.state != *g.header.spec().state.as_bytes()
            || invite.inviter != seed.context.spec_data().signer
            || seed.metadata.routing_label != 0
            || seed.metadata.routing.len() != 1
            || seed.metadata.routing[0].0 != 0
        {
            return Err(unavailable());
        }
        let policy = seed.metadata.policy.as_ref().ok_or_else(unavailable)?;
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
}

impl FrozenAddRequest {
    pub(super) fn prepare(
        seed: &SeedBody,
        installed: Option<V2InstalledFounderAddData>,
        data: V2FounderAddRequestData<'_>,
    ) -> Result<Self, SyncError> {
        let policy = data.verify_seed(seed)?;
        let g = &seed.genesis;
        let invite = data.invite.spec_data();
        // The wrapper admits Add only. No Remove or policy mutation can make
        // these original L0 secrets stale. Future transition support MUST
        // install genuine updated routing custody, never rederive L0 from a
        // post-Add exporter (only the transfer wrapping key changes on Add).
        let spec = data.request.spec_data().clone();
        let before_epoch = seed.group.epoch();
        let after_epoch = before_epoch.checked_add(1).ok_or_else(unavailable)?;
        let parent = installed.map_or(g.header.id(), |s| s.header);
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
        field(&mut plain, seed.metadata.routing[0].1.as_ref())?;
        field(&mut plain, seed.metadata._file_wrap_key.as_ref())?;
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
