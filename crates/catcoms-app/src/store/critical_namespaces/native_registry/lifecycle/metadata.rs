//! Closed request routing within ONE retained metadata cell. A request tag is
//! part of retry equality; equal payload bytes across operations are not hits.
use super::{
    data::{Change, Index},
    refresh, rename, *,
};

#[derive(PartialEq, Eq)]
pub(super) enum Request {
    Rename(rename::Request),
    Refresh(refresh::Request),
}
#[derive(Clone, Copy)]
pub(super) enum Input<'a> {
    Rename(&'a rename::Request),
    Refresh(&'a refresh::Request),
}
// Exactly one frozen request payload in the same existing control invoice,
// separate from selected/candidate strings and ciphertext/AEAD terms.
pub(super) const RETAINED_PAYLOAD: usize = super::super::data::MAX_INVITE;
const _: () = assert!(super::super::data::MAX_NAME <= RETAINED_PAYLOAD);
impl Input<'_> {
    pub(super) fn require_bounded(self) -> Result<(), AppError> {
        let fits = match self {
            Self::Rename(r) => r.name.len() <= super::super::data::MAX_NAME,
            Self::Refresh(r) => r.invite.len() <= super::super::data::MAX_INVITE,
        };
        if fits {
            Ok(())
        } else {
            Err(refused())
        }
    }
    pub(super) fn matches(self, retained: &Request) -> bool {
        match (self, retained) {
            (Self::Rename(a), Request::Rename(b)) => a == b,
            (Self::Refresh(a), Request::Refresh(b)) => a == b,
            _ => false,
        }
    }
    pub(super) fn freeze(self) -> Request {
        match self {
            Self::Rename(r) => Request::Rename(r.clone()),
            Self::Refresh(r) => Request::Refresh(r.clone()),
        }
    }
    pub(super) fn kind(self) -> Change {
        match self {
            Self::Rename(_) => Change::Rename,
            Self::Refresh(_) => Change::RefreshLegacyInvite,
        }
    }
    pub(super) fn prepare(
        self,
        selected: Index,
        parent: [u8; 32],
    ) -> Result<rename::Plan, AppError> {
        match self {
            Self::Rename(r) => rename::prepare(selected, r, parent),
            Self::Refresh(r) => refresh::prepare(selected, r, parent),
        }
    }
}
