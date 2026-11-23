//! Public membership data from one checked native selected prefix.
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

pub struct NativeTextRoster {
    members: Vec<NativeTextRosterMember>,
    _payment: CatalogReadReservation,
}

impl NativeTextRoster {
    pub fn members(&self) -> &[NativeTextRosterMember] {
        &self.members
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
            _payment: payment,
        })
    }
}
