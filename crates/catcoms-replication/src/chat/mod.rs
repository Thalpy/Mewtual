//! Inactive typed chat core. This is a separate semantic action DAG, not a legacy
//! Automerge change adapter. No DocType, network/native/UI producer or authority issuer
//! is connected. In particular this module does NOT close the production R6 residual.
//!
//! Historical authorization is supplied as an opaque, exact-scope witness. Decoding a
//! signature, snapshot, context ID or a peer's "verified" flag never creates that witness.
//! Governance/root continuity, conflicting succession and old-context moderation policy
//! remain unresolved production boundaries; this core does not decide them by arrival.
mod action;
mod authority;
mod ledger;
mod projection;
mod storage;

pub use action::{ActionSpec, Body, Realm, SignedAction};
pub use authority::{Rights, VerifiedAuthorityBasis, VerifiedDeleteApproval};
pub use ledger::{ActionState, Advance, ChatLedger, ImportOutcome, LedgerLimits, PendingReason};
pub use projection::{EditAlternative, MessageProjection};
pub use storage::{ChatSnapshot, Page, PageCursor, MAX_PAGE_BYTES, MAX_PAGE_RECORDS};

/// Fixed grammar bounds, not a new MLS membership/group-size limit. Total retained
/// history is separately bounded by the caller's explicit LedgerLimits, never evicted.
pub const MAX_PARENTS: usize = 64;
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_ACTION_BYTES: usize = 128 * 1024;
pub const MAX_GROUP_ID_BYTES: usize = 256;
/// Fixed in-memory optional approval slot reserved by every retained action entry. The
/// existing `LedgerLimits::max_actions` quota covers at most one such slot per action;
/// this is a logical structural charge, not an allocator/RSS or native payer invoice.
/// It is not encoded or included in canonical action-envelope byte accounting.
pub const DELETE_APPROVAL_ENTRY_BYTES: usize =
    std::mem::size_of::<Option<VerifiedDeleteApproval>>();
pub const PROFILE: u16 = 1;

macro_rules! digest {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 32]);
        impl $name {
            /// Public immutable label only; this never supplies authority.
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }
    };
}
digest!(RealmId);
digest!(ActionId);
digest!(BasisId);
digest!(GrantId);
digest!(EnvelopeHash);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChatError {
    #[error("malformed typed chat frame")]
    Malformed,
    #[error("typed chat frame exceeds its bound")]
    Bound,
    #[error("typed chat signature is invalid")]
    Signature,
    #[error("typed chat scope does not match")]
    Scope,
    #[error("typed chat authority does not match")]
    Authority,
    #[error("typed chat authority witnesses conflict")]
    AuthorityConflict,
    #[error("typed chat retained capacity is full")]
    Capacity,
    #[error("typed chat dependency or target is invalid")]
    Dependency,
    #[error("typed chat action is not authorized")]
    Permission,
    #[error("typed chat bounded work allowance is exhausted")]
    WorkLimit,
    #[error("typed chat action is not known")]
    Unknown,
}
impl From<catcoms_wire::WireError> for ChatError {
    fn from(_: catcoms_wire::WireError) -> Self {
        Self::Malformed
    }
}

#[cfg(test)]
mod tests;
