//! Public membership data from one checked native selected prefix.
catcoms_rt::redacted_debug! {
    NativeTextRoster;
}

use super::*;
use catcoms_replication::chat::{
    owner_header::OwnerBodyId, owner_profile::MAX_CURRENT_MEMBERS,
    owner_state::transition::ReplayMath,
};
use critical_namespaces::{read_pool::CatalogReadReservation, refused};

#[derive(Clone, Copy, Debug)]
pub struct NativeTextRosterMember {
    pub admission: OwnerBodyId,
    pub signer: [u8; 32],
    pub founder: bool,
    pub you: bool,
}

impl NativeTextRosterMember {
    pub fn device_id(&self) -> catcoms_crypto::DeviceId {
        catcoms_crypto::DeviceId::from_public_key_bytes(&self.signer)
    }
}

pub struct NativeTextRoster {
    members: Vec<NativeTextRosterMember>,
    can_remove: bool,
    can_remember_founder_route: bool,
    _payment: CatalogReadReservation,
}

impl NativeTextRoster {
    pub fn members(&self) -> &[NativeTextRosterMember] {
        &self.members
    }

    pub fn can_remove(&self) -> bool {
        self.can_remove
    }

    /// Whether this is an eligible original A1 row. The actual consent
    /// command still proves the selected invite and exact outbound route.
    pub fn can_remember_founder_route(&self) -> bool {
        self.can_remember_founder_route
    }

    pub(super) fn with_founder_route_option(mut self, available: bool) -> Self {
        self.can_remember_founder_route = available;
        self
    }

    pub(super) fn with_founder_removal(mut self) -> Self {
        self.can_remove = true;
        self
    }

    pub(super) fn from_selected(
        store: &ServerStore,
        replay: &ReplayMath,
        own: OwnerBodyId,
    ) -> Result<Self, AppError> {
        replay.slot(own).ok_or_else(refused)?;
        let founder = replay.profile().spec().founder;
        Self::prepare(
            store,
            replay.slots().map(|slot| NativeTextRosterMember {
                admission: slot.image.admission,
                signer: slot.image.subject,
                founder: slot.image.subject == founder,
                you: slot.image.admission == own,
            }),
        )
    }

    pub(super) fn prepare(
        store: &ServerStore,
        members: impl ExactSizeIterator<Item = NativeTextRosterMember>,
    ) -> Result<Self, AppError> {
        let count = members.len();
        if !(1..=usize::from(MAX_CURRENT_MEMBERS)).contains(&count) {
            return Err(refused());
        }
        let pool = &store
            .governance_layout_policy
            .as_ref()
            .ok_or_else(refused)?
            .current
            .archive
            .pending
            .pool;
        let bytes = 4
            * (count * std::mem::size_of::<NativeTextRosterMember>() + std::mem::size_of::<Self>())
            + 1024;
        let payment = pool.try_reserve(bytes)?;
        let members: Vec<_> = members.collect();
        if members.len() != count {
            return Err(refused());
        }
        payment.validate(pool, bytes)?;
        Ok(Self {
            members,
            can_remove: false,
            can_remember_founder_route: false,
            _payment: payment,
        })
    }
}
