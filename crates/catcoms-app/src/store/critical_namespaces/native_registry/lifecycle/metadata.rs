//! Closed request routing within ONE retained metadata cell. A request tag is
//! part of retry equality; equal payload bytes across operations are not hits.
use super::{
    data::{Change, Index},
    founder, refresh, rename, *,
};

#[derive(PartialEq, Eq)]
pub(super) enum Request {
    Rename(rename::Request),
    Refresh(refresh::Request),
    FounderBirth(founder::Pending),
    FounderActivate(founder::Activate),
    NativeFounderActivate(founder::Activate),
}
#[derive(Clone, Copy)]
pub(super) enum Input<'a> {
    Rename(&'a rename::Request),
    Refresh(&'a refresh::Request),
    FounderBirth(&'a founder::Pending),
    FounderActivate(&'a founder::Activate),
    NativeFounderActivate(
        &'a founder::Activate,
        &'a std::sync::Arc<crate::store::persistence::native_registry::network::NetworkDependency>,
    ),
}
// Exactly one frozen request payload in the same existing control invoice,
// separate from selected/candidate strings and ciphertext/AEAD terms.
pub(super) const RETAINED_PAYLOAD: usize =
    super::super::data::MAX_INVITE + super::super::data::MAX_NAME + 256;
// FounderBirth retains one bounded network frame plus its original local name.
// Activation retains at most a 256-byte group. Both share this same paid cell;
// no additional concurrent transaction or uncharged index copy is introduced.
const _: () = assert!(founder::MAX_NETWORK_FRAME <= super::super::data::MAX_INVITE);

#[derive(Clone, Copy)]
pub(super) enum Outcome {
    Metadata(rename::Metadata),
    Creation(crate::store::NativeCreationStatusData),
}
pub(super) enum Plan {
    NoChange(Outcome),
    Change {
        selected: Box<Index>,
        candidate: Box<Index>,
        result: Outcome,
    },
}
impl From<rename::Plan> for Plan {
    fn from(plan: rename::Plan) -> Self {
        match plan {
            rename::Plan::NoChange(result) => Self::NoChange(Outcome::Metadata(result)),
            rename::Plan::Change {
                selected,
                candidate,
                result,
            } => Self::Change {
                selected,
                candidate,
                result: Outcome::Metadata(result),
            },
        }
    }
}
const _: () = assert!(super::super::data::MAX_NAME <= RETAINED_PAYLOAD);
impl<'a> Input<'a> {
    pub(super) fn require_bounded(self) -> Result<(), AppError> {
        match self {
            Self::FounderBirth(r) => return r.require_bounded(),
            Self::FounderActivate(r) => {
                if r.birth.kind != super::data::BirthKind::Founder {
                    return Err(refused());
                }
                return r.require_bounded();
            }
            Self::NativeFounderActivate(r, _) => {
                if r.birth.kind != super::data::BirthKind::NativeFounder {
                    return Err(refused());
                }
                return r.require_bounded();
            }
            _ => (),
        }
        let fits = match self {
            Self::Rename(r) => r.name.len() <= super::super::data::MAX_NAME,
            Self::Refresh(r) => r.invite.len() <= super::super::data::MAX_INVITE,
            Self::FounderBirth(_) | Self::FounderActivate(_) | Self::NativeFounderActivate(..) => {
                unreachable!()
            }
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
            (Self::FounderBirth(a), Request::FounderBirth(b)) => a == b,
            (Self::FounderActivate(a), Request::FounderActivate(b)) => a == b,
            (Self::NativeFounderActivate(a, _), Request::NativeFounderActivate(b)) => a == b,
            _ => false,
        }
    }
    pub(super) fn freeze(self) -> Request {
        match self {
            Self::Rename(r) => Request::Rename(r.clone()),
            Self::Refresh(r) => Request::Refresh(r.clone()),
            Self::FounderBirth(r) => Request::FounderBirth(r.clone()),
            Self::FounderActivate(r) => Request::FounderActivate(r.clone()),
            Self::NativeFounderActivate(r, _) => Request::NativeFounderActivate(r.clone()),
        }
    }
    pub(super) fn kind(self) -> Change {
        match self {
            Self::Rename(_) => Change::Rename,
            Self::Refresh(_) => Change::RefreshLegacyInvite,
            Self::FounderBirth(_) => Change::Birth,
            Self::FounderActivate(_) | Self::NativeFounderActivate(..) => Change::Activate,
        }
    }
    pub(super) fn prepare(self, selected: Index, parent: [u8; 32]) -> Result<Plan, AppError> {
        match self {
            Self::Rename(r) => rename::prepare(selected, r, parent).map(Into::into),
            Self::Refresh(r) => refresh::prepare(selected, r, parent).map(Into::into),
            Self::FounderBirth(r) => r.prepare(selected, parent),
            Self::FounderActivate(r) => r.prepare(selected, parent),
            Self::NativeFounderActivate(r, _) => r.prepare(selected, parent),
        }
    }
    pub(super) fn network(
        self,
    ) -> Option<
        &'a std::sync::Arc<crate::store::persistence::native_registry::network::NetworkDependency>,
    > {
        match self {
            Self::NativeFounderActivate(_, proof) => Some(proof),
            _ => None,
        }
    }
}
