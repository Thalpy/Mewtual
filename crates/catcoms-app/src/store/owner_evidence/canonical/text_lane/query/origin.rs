//! Closed actual query families sharing bounded native reader accounting.
//!
//! These enums are PRIVATE to this kernel. Each variant requires the corresponding
//! concrete physical producer's opaque observation/fee/claim. No caller-selected tree,
//! parsed root, generic trait or resource adopter constructs a generation or a reader.
use super::*;
use crate::store::persistence::v2_text::{
    OriginalV2TextAcceptedQueryObservation, V2TextAcceptedQueryReaderClaim,
    V2TextAcceptedQueryResources,
};

#[derive(Clone)]
pub(super) enum QueryOrigin {
    Token(OriginalV2TextTokenQueryObservation),
    Accepted(OriginalV2TextAcceptedQueryObservation),
}
impl QueryOrigin {
    // Cached actual selected-O identities from the two concrete paid issuers.
    pub(super) fn selected_head_id_data(
        &self,
    ) -> crate::store::persistence::v2_text::TextHeadIdData {
        match self {
            Self::Token(value) => value.selected_head_id_data(),
            Self::Accepted(value) => value.selected_head_id_data(),
        }
    }
    pub(super) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        match (self, other) {
            (Self::Token(a), Self::Token(b)) => a.require_same_original(b),
            (Self::Accepted(a), Self::Accepted(b)) => a.require_same_original(b),
            _ => Err(refused()),
        }
    }
    pub(super) fn require_same_selected_parent(&self, other: &Self) -> Result<(), AppError> {
        match (self, other) {
            (Self::Token(a), Self::Token(b)) => a.require_same_selected_parent(b),
            (Self::Accepted(a), Self::Accepted(b)) => a.require_same_selected_parent(b),
            // Even equal O/H bytes do not turn a token-tree result into an accepted
            // Directory/Actions query. Their physical issuers and contexts differ.
            _ => Err(refused()),
        }
    }
    pub(super) fn require_token_kind(&self) -> Result<(), AppError> {
        match self {
            Self::Token(_) => Ok(()),
            Self::Accepted(_) => Err(refused()),
        }
    }
    pub(super) fn require_token_original(
        &self,
        original: &OriginalV2TextTokenQueryObservation,
    ) -> Result<(), AppError> {
        match self {
            Self::Token(actual) => actual.require_same_original(original),
            Self::Accepted(_) => Err(refused()),
        }
    }
    pub(super) fn require_accepted_original(
        &self,
        original: &OriginalV2TextAcceptedQueryObservation,
    ) -> Result<(), AppError> {
        match self {
            Self::Accepted(actual) => actual.require_same_original(original),
            Self::Token(_) => Err(refused()),
        }
    }
}

/// Each generation/result retains its OWN producer's independent query fee. The
/// original selected-source and SDK payments remain additional; no variant converts
/// one payment into another or supplies private-root/lookup authority by itself.
#[derive(Clone)]
pub(super) enum QueryResourcesOrigin {
    Token(V2TextTokenQueryResources),
    Accepted(V2TextAcceptedQueryResources),
}
impl QueryResourcesOrigin {
    pub(super) fn validate_prepaid_account(&self) -> Result<(), AppError> {
        match self {
            Self::Token(resources) => resources.validate_prepaid_account(),
            Self::Accepted(resources) => resources.validate_prepaid_account(),
        }
    }
}

/// Exactly TWO private aliases: prepared wrapper plus its real affine reader. The
/// underlying producer supplies the maxONE preparation claim BEFORE any allocation.
/// Keeping it after both wrapper metadata and the actual End prevents a second paid
/// precursor from renewing while either old tail is still alive.
pub(super) enum QueryReaderClaimAlias {
    Token(Arc<V2TextTokenQueryReaderClaim>),
    Accepted(Arc<V2TextAcceptedQueryReaderClaim>),
}
impl Clone for QueryReaderClaimAlias {
    fn clone(&self) -> Self {
        match self {
            Self::Token(claim) => Self::Token(Arc::clone(claim)),
            Self::Accepted(claim) => Self::Accepted(Arc::clone(claim)),
        }
    }
}
