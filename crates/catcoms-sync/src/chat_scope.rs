//! Process-local fresh admission evidence. This is not historical membership authority or a
//! decoded snapshot capability. The vault host must still couple it to its ORIGINAL pre-await
//! mount/lineage capture before it can renew a local authoring scope.
use super::*;
mod founder_origin;
pub(crate) use founder_origin::OriginalFounderObservation;

#[derive(Clone, Copy)]
enum FreshKind {
    Found {
        authenticator: [u8; 32],
    },
    Welcome {
        key_package: [u8; 32],
        welcome_hash: [u8; 32],
    },
}

/// Opaque, nonserialized completion of actual fresh creation or an authenticated exact-KP join.
/// No Clone, wire/snapshot decoder or public field constructor exists. A supplied digest, raw
/// Welcome parser, epoch-zero restored group or RoutingState::default cannot mint this value.
pub struct VerifiedFreshChatAdmission {
    group: Vec<u8>,
    signer: [u8; 32],
    // Bind the handoff to the ORIGINAL MLS admission, including same-key rejoins. Group/key
    // equality alone would let a caller attach routing A1's proof to restored/new admission A2.
    admission_state: [u8; 32],
    kind: FreshKind,
    attempt: [u8; 32],
    // Process-local original source lifetime only. Neither identity nor watch is serialized;
    // an equal restored MLS state cannot acquire them. Observers never retain this owner.
    origin: founder_origin::FreshOriginLife,
}
impl std::fmt::Debug for VerifiedFreshChatAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Do not expose signer, group, admission/attempt identifiers or Welcome material.
        f.debug_struct("VerifiedFreshChatAdmission")
            .finish_non_exhaustive()
    }
}
impl VerifiedFreshChatAdmission {
    /// Observe this actual original creation, not a label reconstructed from current state.
    /// Keeping the original token PRESENT in the same Sync remains mandatory at later checks.
    pub(super) fn observe_original_founder(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
    ) -> Option<OriginalFounderObservation> {
        self.is_original_founder_state(group, device)
            .then(|| self.origin.observe())
    }
    /// Narrow genesis-producer predicate. Generic local authoring provenance can survive
    /// unrelated epochs, but installing CHECKPOINT-1 cannot reconstruct past Adds from a
    /// current roster. Require this actual creation's ORIGINAL authenticated state instead.
    pub(super) fn is_original_founder_state(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
    ) -> bool {
        matches!(self.kind, FreshKind::Found { .. }) && self.matches_initial_state(group, device)
    }
    pub(super) fn matches_initial_state(&self, group: &ServerGroup, device: &MlsDevice) -> bool {
        self.group == group.group_id()
            && self.signer.as_slice() == device.public_key_bytes()
            && self.admission_state == group.epoch_authenticator_id()
            && group.is_active()
            && group.has_local_signer(&device.public_key_bytes())
    }
    pub fn group(&self) -> &[u8] {
        &self.group
    }
    pub fn signer(&self) -> &[u8; 32] {
        &self.signer
    }
    pub fn attempt_id(&self) -> [u8; 32] {
        self.attempt
    }
    pub fn key_package_reference(&self) -> Option<[u8; 32]> {
        match self.kind {
            FreshKind::Welcome { key_package, .. } => Some(key_package),
            FreshKind::Found { .. } => None,
        }
    }
    pub fn authenticated_welcome_hash(&self) -> Option<[u8; 32]> {
        match self.kind {
            FreshKind::Welcome { welcome_hash, .. } => Some(welcome_hash),
            FreshKind::Found { .. } => None,
        }
    }

    // Only the wrapper which actually invokes ServerGroup::create calls this constructor.
    pub(super) fn from_fresh_creation(
        group: &ServerGroup,
        device: &MlsDevice,
    ) -> Result<Self, SyncError> {
        Self::complete(
            group,
            device,
            FreshKind::Found {
                authenticator: group.epoch_authenticator_id(),
            },
        )
    }

    // Call AFTER inviter/owner signature, exact selected KP, group and routing/policy checks.
    pub(super) fn from_verified_welcome(
        group: &ServerGroup,
        device: &MlsDevice,
        key_package: [u8; 32],
        welcome: &[u8],
    ) -> Result<Self, SyncError> {
        Self::complete(
            group,
            device,
            FreshKind::Welcome {
                key_package,
                welcome_hash: *blake3::hash(welcome).as_bytes(),
            },
        )
    }

    fn complete(
        group: &ServerGroup,
        device: &MlsDevice,
        kind: FreshKind,
    ) -> Result<Self, SyncError> {
        if !group.is_active() || !group.has_local_signer(&device.public_key_bytes()) {
            return Err(SyncError::JoinRejected);
        }
        let group_state = group.epoch_authenticator_id();
        let group = group.group_id();
        if group.is_empty() || group.len() > 256 {
            return Err(SyncError::Malformed);
        }
        let signer: [u8; 32] = device
            .public_key_bytes()
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        let mut hash = blake3::Hasher::new();
        hash.update(b"catcoms/local-chat-fresh-admission/v1\0");
        hash.update(&(group.len() as u64).to_be_bytes());
        hash.update(&group);
        hash.update(&signer);
        match kind {
            FreshKind::Found { authenticator } => {
                hash.update(&[0]);
                hash.update(&authenticator);
            }
            FreshKind::Welcome {
                key_package,
                welcome_hash,
            } => {
                hash.update(&[1]);
                hash.update(&key_package);
                hash.update(&welcome_hash);
            }
        }
        Ok(Self {
            group,
            signer,
            admission_state: group_state,
            kind,
            attempt: *hash.finalize().as_bytes(),
            origin: founder_origin::FreshOriginLife::new(),
        })
    }
}

#[cfg(test)]
mod owner_profile_origin_tests;
