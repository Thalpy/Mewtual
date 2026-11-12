//! Join FACTS for a genuinely joined ordinary member's Chat V2 source.
//!
//! These are the historical facts the App's joined-member source needs to bind the member's
//! own signed V2 Add to THIS device's actual join:
//! - the consumed KeyPackage reference (the same OpenMLS `hash_ref` encoding as
//!   `LocalMembershipChange::key_package_reference`);
//! - the own credential digest (the same domain as an Add observation's `credential`);
//! - the join epoch;
//! - the MLS admission state (epoch authenticator) right after the Welcome.
//!
//! The Add's signed fields are commit, epochs, `key_package` and `credential`. The joiner never
//! saw its own Add's commit bytes, so that digest stays unverifiable here, and the admission
//! state is a LOCAL association only, never signed evidence.
//!
//! What this is not: V2 pin provenance (the trust root is the primary's mounted joined pin), a
//! membership proof, a selected prefix, or any signing, save or publication permission.
//!
//! Lifecycle on ONE node incarnation (see [`JoinFactsSlot`]):
//! - `finish_join` captures them from the exact verified invite Welcome, before any later
//!   transition, and carries them in the transient, never-encoded `RoutingState`. A
//!   companion-device join captures none.
//! - `new_joined` keeps them only for the exact group/device/state they were captured on.
//! - The joined source takes them AT MOST ONCE. After that the v1 owner checkpoint install
//!   refuses on this node. If the v1 install runs first, they can never be taken. The two
//!   owners of this node's membership story are mutually exclusive, whichever comes first.
//! - A restored node never has them: they are not serialized.
use crate::owner_v2::V2MemberSignRefusal;
use crate::*;

/// Exact join facts captured at the verified Welcome. Not Clone: one owner takes them once.
pub struct V2JoinFacts {
    group: Vec<u8>,
    signer: [u8; 32],
    key_package: [u8; 32],
    credential: [u8; 32],
    join_epoch: u64,
    admission_state: [u8; 32],
}

impl std::fmt::Debug for V2JoinFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never log the signer, KeyPackage reference, credential digest or admission state.
        f.debug_struct("V2JoinFacts").finish_non_exhaustive()
    }
}

impl V2JoinFacts {
    /// Capture on the exact just-joined group. The caller has already verified the inviter
    /// signature, the exact selected KeyPackage, the group id and the routing/policy transfer.
    /// Must run before any later transition: `own_credential_digest` needs the active own leaf.
    pub(crate) fn capture(
        group: &ServerGroup,
        device: &MlsDevice,
        key_package: [u8; 32],
    ) -> Result<Self, SyncError> {
        if !group.is_active() || !group.has_local_signer(&device.public_key_bytes()) {
            return Err(SyncError::JoinRejected);
        }
        let signer: [u8; 32] = device
            .public_key_bytes()
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        let credential = group
            .own_credential_digest(device)
            .map_err(|_| SyncError::JoinRejected)?;
        let id = group.group_id();
        if id.is_empty() || id.len() > 256 {
            return Err(SyncError::Malformed);
        }
        Ok(Self {
            group: id,
            signer,
            key_package,
            credential,
            join_epoch: group.epoch(),
            admission_state: group.epoch_authenticator_id(),
        })
    }

    /// The exact-handoff check `new_joined` applies. `RoutingState` is a public transport
    /// container that could be paired with another raw group/device. Facts are kept only for
    /// the very group, device and post-Welcome state they were captured on, as with
    /// `VerifiedFreshChatAdmission::matches_initial_state`.
    pub(crate) fn matches_initial_state(&self, group: &ServerGroup, device: &MlsDevice) -> bool {
        self.group == group.group_id()
            && self.signer.as_slice() == device.public_key_bytes().as_slice()
            && self.join_epoch == group.epoch()
            && self.admission_state == group.epoch_authenticator_id()
    }

    pub fn group_data(&self) -> &[u8] {
        &self.group
    }
    /// This device's full retained signature key at join: the own Add's required `subject`.
    pub fn signer_data(&self) -> &[u8; 32] {
        &self.signer
    }
    /// The consumed KeyPackage reference: the own Add's required `key_package`.
    pub fn key_package_data(&self) -> &[u8; 32] {
        &self.key_package
    }
    /// The own credential digest at join: the own Add's required `credential`.
    pub fn credential_data(&self) -> &[u8; 32] {
        &self.credential
    }
    /// The epoch this Welcome put the node in: the own Add's required `after_epoch`.
    pub fn join_epoch_data(&self) -> u64 {
        self.join_epoch
    }
    /// LOCAL association only (not in any signed record): the epoch authenticator right after
    /// the Welcome, for the joined source's same-epoch state comparison.
    pub fn admission_state_data(&self) -> &[u8; 32] {
        &self.admission_state
    }
}

/// Join-facts custody on one node incarnation. Never serialized.
#[derive(Default)]
pub(crate) enum JoinFactsSlot {
    /// Not a verified invite join on this incarnation: a founder, restored or companion node.
    #[default]
    Absent,
    Available(Box<V2JoinFacts>),
    /// Taken by the joined-member source. The v1 owner checkpoint install now refuses.
    TakenByJoinedSource,
    /// The v1 owner checkpoint install ran. Join facts can never be taken on this node.
    BoundToV1Owner,
}

impl JoinFactsSlot {
    pub(crate) fn from_handoff(facts: Option<V2JoinFacts>) -> Self {
        match facts {
            Some(facts) => Self::Available(Box::new(facts)),
            None => Self::Absent,
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Move this ORIGINAL joined incarnation's join facts out, exactly once, to the App's
    /// joined-member source, together with the source's handle on this node's live fence. The
    /// fence has recorded every finalized transition since the Welcome, unless it already
    /// closed (for example with `HistoryGap`); check `observe_data`. Refuses for another
    /// incarnation, for a node with no facts (founder, restored, companion device, or a
    /// mismatched handoff), for facts already taken, and after the v1 owner checkpoint install
    /// bound this node. A refusal changes nothing.
    ///
    /// DATA plus ordering custody only. Taking them is not membership, selection or signing
    /// permission; the source must still establish the mounted pin and complete replay.
    pub fn take_v2_joined_join_facts(
        &mut self,
        instance: &RegistrySyncInstance,
    ) -> Result<(V2JoinFacts, crate::owner_v2::V2JoinedSourceFence), V2MemberSignRefusal> {
        if !self.matches_registry_instance(instance)
            || !matches!(self.v2_join_facts, JoinFactsSlot::Available(_))
        {
            return Err(V2MemberSignRefusal::Source);
        }
        let fence = self.v2_fence.handle().ok_or(V2MemberSignRefusal::Source)?;
        match std::mem::replace(&mut self.v2_join_facts, JoinFactsSlot::TakenByJoinedSource) {
            JoinFactsSlot::Available(facts) => Ok((*facts, fence)),
            other => {
                self.v2_join_facts = other;
                Err(V2MemberSignRefusal::Source)
            }
        }
    }

    /// The v1 owner checkpoint install's exclusion check. It runs BEFORE that install takes
    /// or replaces anything, so a refusal leaves the whole candidate in place.
    pub(crate) fn require_no_v2_joined_source(&self) -> Result<(), SyncError> {
        match self.v2_join_facts {
            JoinFactsSlot::TakenByJoinedSource => Err(SyncError::ChatScopeUnavailable),
            JoinFactsSlot::Absent | JoinFactsSlot::Available(_) | JoinFactsSlot::BoundToV1Owner => {
                Ok(())
            }
        }
    }

    /// Called only after a successful v1 owner checkpoint install. Any untaken join facts are
    /// dropped, the node's fence closes for good (that install bypassed the transition
    /// observer), and the joined source can never be created on this incarnation.
    pub(crate) fn note_v1_owner_install(&mut self) {
        self.v2_join_facts = JoinFactsSlot::BoundToV1Owner;
        self.v2_fence
            .terminate(crate::owner_v2::V2JoinedFenceTerminal::V1Owner);
    }
}

#[cfg(test)]
mod tests;
