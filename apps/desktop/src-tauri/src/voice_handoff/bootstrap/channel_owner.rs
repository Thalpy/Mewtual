//! Immutable physical-channel provenance. Shared metadata never supplies a grant
//! or converts an admitted key into the Existing pilot/ledger authority.
use super::*;
use crate::voice_authority::{AdmittedPublishedKey, Binding};
use catcoms_sync::voice_v2::CallLease;

#[derive(Clone)]
pub(in crate::voice_handoff) enum ChannelOwner {
    Existing(Arc<SelectedHandoffOwner>),
    Admitted(AdmittedPublishedKey),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::voice_handoff) enum ChannelProfile {
    Existing,
    Admitted,
}
impl ChannelOwner {
    pub(in crate::voice_handoff) fn profile(&self) -> ChannelProfile {
        match self {
            Self::Existing(_) => ChannelProfile::Existing,
            Self::Admitted(_) => ChannelProfile::Admitted,
        }
    }
    pub(in crate::voice_handoff) fn existing(&self) -> Option<&Arc<SelectedHandoffOwner>> {
        match self {
            Self::Existing(value) => Some(value),
            Self::Admitted(_) => None,
        }
    }
    pub(in crate::voice_handoff) fn admitted(&self) -> Option<&AdmittedPublishedKey> {
        match self {
            Self::Admitted(value) => Some(value),
            Self::Existing(_) => None,
        }
    }
    pub(in crate::voice_handoff) fn is_same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Existing(a), Self::Existing(b)) => Arc::ptr_eq(a, b),
            (Self::Admitted(a), Self::Admitted(b)) => a.is_same(b),
            _ => false,
        }
    }
    pub(in crate::voice_handoff) fn id(&self) -> &str {
        match self {
            Self::Existing(value) => value.id(),
            Self::Admitted(value) => value.id(),
        }
    }
    pub(in crate::voice_handoff) fn binding(&self) -> &Binding {
        match self {
            Self::Existing(value) => value.binding(),
            Self::Admitted(value) => value.binding(),
        }
    }
    pub(in crate::voice_handoff) fn join(&self) -> [u8; 32] {
        match self {
            Self::Existing(value) => value.join(),
            Self::Admitted(value) => value.join(),
        }
    }
    pub(in crate::voice_handoff) fn executor(&self) -> [u8; 32] {
        match self {
            Self::Existing(value) => value.executor(),
            Self::Admitted(value) => value.executor(),
        }
    }
    pub(in crate::voice_handoff) fn point(&self) -> [u8; 65] {
        match self {
            Self::Existing(value) => value.point(),
            Self::Admitted(value) => value.point(),
        }
    }
    pub(in crate::voice_handoff) fn lease(&self) -> &CallLease {
        match self {
            Self::Existing(value) => value.lease(),
            Self::Admitted(value) => value.lease(),
        }
    }
    pub(in crate::voice_handoff) fn resources(&self) -> Arc<Resources> {
        match self {
            Self::Existing(value) => value.resources(),
            Self::Admitted(value) => value.resources(),
        }
    }
}
