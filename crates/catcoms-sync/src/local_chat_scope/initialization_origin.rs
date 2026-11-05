//! Private initialization source observation, never a scope/Current/publication constructor.
use super::*;

/// Retain only inert identity plus ORIGINAL negative custody. No sender/revoker or runtime
/// owner is kept alive. A replacement scope must be prepared anew before signing/freeze.
pub(crate) struct OriginalInitializationObservation {
    required: bool,
    label: Option<ChatScopeLabel>,
    revision: u64,
    reservation: Option<(Arc<()>, RequestCancellation)>,
}

fn private_shape(scope: &LocalChatScope) -> bool {
    if scope.runtime.is_some()
        || !scope.aliases.is_empty()
        || !scope.recovery.is_empty()
        || scope.revision == u64::MAX
    {
        return false;
    }
    match &scope.label {
        None => !scope.required && scope.reservation.is_none(),
        Some(label) => {
            !label.retired
                && !label.initializing
                && scope
                    .reservation
                    .as_ref()
                    .is_none_or(|source| !source.observer.is_cancelled())
        }
    }
}
impl LocalChatScope {
    pub(crate) fn observe_private_initialization(
        &self,
    ) -> Result<OriginalInitializationObservation, SyncError> {
        if !private_shape(self) {
            return Err(SyncError::ChatScopeUnavailable);
        }
        Ok(OriginalInitializationObservation {
            required: self.required,
            label: self.label.clone(),
            revision: self.revision,
            reservation: self
                .reservation
                .as_ref()
                .map(|source| (source.identity.clone(), source.observer.clone())),
        })
    }
}
impl OriginalInitializationObservation {
    /// Original managed-domain DATA for a private archive producer, never a mounted key,
    /// Current or storage/publication permit. A staged scalar label without its ORIGINAL live
    /// reservation is insufficient, even when its group/domain/nonce happen to match.
    /// Callers still repeat their complete actual source checks and hold real host admission
    /// through final save; cancellation can happen immediately after this observation returns.
    pub(crate) fn managed_archive_domain(
        &self,
        scope: &LocalChatScope,
    ) -> Result<[u8; 32], SyncError> {
        if !self.matches(scope) || !self.required || self.reservation.is_none() {
            return Err(SyncError::ChatScopeUnavailable);
        }
        self.label
            .as_ref()
            .map(|label| label.domain)
            .ok_or(SyncError::ChatScopeUnavailable)
    }

    pub(crate) fn matches(&self, scope: &LocalChatScope) -> bool {
        private_shape(scope)
            && self.required == scope.required
            && self.label == scope.label
            && self.revision == scope.revision
            && match (&self.reservation, &scope.reservation) {
                (None, None) => true,
                (Some((identity, negative)), Some(current)) => {
                    !negative.is_cancelled() && Arc::ptr_eq(identity, &current.identity)
                }
                _ => false,
            }
    }
}
