//! The public current-owner facts that repair and adoption authority checks read, behind one
//! sealed trait, so a detached worker can run those exact checks with no MLS state (Agent 3
//! design 10.3, stage S2).
//!
//! Every authority check on the repair and adoption paths (`Receipt::verify_current_owner`,
//! `ReceiptRepair::verify_current_owner`, `from_checkpoint`, and the receipt-book and owner-journal
//! resolvers built on them) reads exactly four facts: the group id, the MLS epoch, the designated
//! committer, and that committer's signature key. Each check first requires the signer to BE the
//! designated committer and only then asks for its key, so a view that answers the key for the
//! committer alone returns the same verdict as the live group. Anything else asking a captured
//! view for another member's key gets `None` and fails closed.
//!
//! A [`CapturedOwnerAuthority`] is a snapshot, never a lease. Whoever commits work planned against
//! one must capture again from the live group under custody and require equality first; only then
//! do the verdicts reached in the detached stage still hold.
use catcoms_crypto::DeviceId;
use catcoms_mls::ServerGroup;

mod sealed {
    /// Only the live group and a view captured from it may answer authority questions. A public
    /// trait anyone could implement would let code fabricate an owner the group never had.
    pub trait Sealed {}
    impl Sealed for catcoms_mls::ServerGroup {}
    impl Sealed for super::CapturedOwnerAuthority {}
}

/// The four current-owner facts authority checks may read. Sealed: implemented only by
/// [`ServerGroup`] and [`CapturedOwnerAuthority`].
pub trait OwnerAuthority: sealed::Sealed {
    fn group_id(&self) -> Vec<u8>;
    fn epoch(&self) -> u64;
    fn designated_committer(&self) -> Option<DeviceId>;
    fn member_signature_key(&self, device: &DeviceId) -> Option<Vec<u8>>;
}

impl OwnerAuthority for ServerGroup {
    fn group_id(&self) -> Vec<u8> {
        ServerGroup::group_id(self)
    }
    fn epoch(&self) -> u64 {
        ServerGroup::epoch(self)
    }
    fn designated_committer(&self) -> Option<DeviceId> {
        ServerGroup::designated_committer(self)
    }
    fn member_signature_key(&self, device: &DeviceId) -> Option<Vec<u8>> {
        ServerGroup::member_signature_key(self, device)
    }
}

/// The current-owner facts of one live group at one moment, copied under custody. It holds no
/// MLS secret and can only be built from a real [`ServerGroup`], so it cannot name an owner the
/// group did not have. Equality is how a commit proves nothing it depends on has moved.
#[derive(Clone, PartialEq, Eq)]
pub struct CapturedOwnerAuthority {
    group_id: Vec<u8>,
    epoch: u64,
    /// The designated committer together with its signature key, or `None` for a group with no
    /// committer, in which every authority check refuses.
    committer: Option<(DeviceId, Vec<u8>)>,
}

impl CapturedOwnerAuthority {
    pub fn capture(group: &ServerGroup) -> Self {
        let committer = group
            .designated_committer()
            .and_then(|owner| group.member_signature_key(&owner).map(|key| (owner, key)));
        Self {
            group_id: group.group_id(),
            epoch: group.epoch(),
            committer,
        }
    }
}

impl std::fmt::Debug for CapturedOwnerAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapturedOwnerAuthority")
            .field("epoch", &self.epoch)
            .field(
                "committer",
                &self.committer.as_ref().map(|(owner, _)| owner),
            )
            .finish_non_exhaustive()
    }
}

impl OwnerAuthority for CapturedOwnerAuthority {
    fn group_id(&self) -> Vec<u8> {
        self.group_id.clone()
    }
    fn epoch(&self) -> u64 {
        self.epoch
    }
    fn designated_committer(&self) -> Option<DeviceId> {
        self.committer.as_ref().map(|(owner, _)| *owner)
    }
    fn member_signature_key(&self, device: &DeviceId) -> Option<Vec<u8>> {
        self.committer
            .as_ref()
            .filter(|(owner, _)| owner == device)
            .map(|(_, key)| key.clone())
    }
}

#[cfg(test)]
mod tests;
