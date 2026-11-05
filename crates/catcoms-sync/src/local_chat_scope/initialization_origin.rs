//! Private initialization source observation, never a scope/Current/publication constructor.
use super::*;

/// Retain only inert identity plus ORIGINAL negative custody. No sender/revoker or runtime
/// owner is kept alive. A replacement scope must be prepared anew before signing/freeze.
pub(crate) struct OriginalInitializationObservation {
    required: bool,
    label: Option<ChatScopeLabel>,
    revision: u64,
    reservation: Option<(Arc<()>, RequestCancellation, Option<u64>)>,
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
    /// Receiver-only V2 genesis DATA attachment. The decoded scope has no process lifetime;
    /// retain the SAME trusted host reservation instead of manufacturing publication runtime.
    /// All host fields are captured once, and the durable planned revision must be exact.
    pub(crate) fn attach_genesis_reservation(
        &mut self,
        group: &ServerGroup,
        device: &MlsDevice,
        source: &impl ManagedChatScopeReservation,
        expected_revision: u64,
    ) -> Result<(), SyncError> {
        let domain = source.domain();
        let nonce = source.nonce();
        let revision = source.planned_revision_data();
        let source_group = source.group();
        let signer = *source.signer();
        let identity = source.owner_identity();
        let observer = source.reservation_observer();
        let revoker = source.revocation();
        if !private_shape(self)
            || self.revision != 0
            || !self.required
            || self.label.as_ref().is_none_or(|label| {
                label.domain != domain
                    || label.nonce != nonce
                    || label.retired
                    || label.initializing
            })
            || source_group != group.group_id_ref()
            || signer.as_slice() != device.public_key_bytes()
            || !group.is_active()
            || !group.has_local_signer(&signer)
            || expected_revision == 0
            || revision != Some(expected_revision)
            || observer.is_cancelled()
        {
            return Err(SyncError::ChatScopeUnavailable);
        }
        if let Some(original) = &self.reservation {
            return if Arc::ptr_eq(&original.identity, &identity)
                && !original.observer.is_cancelled()
                && original.planned_revision == revision
            {
                Ok(())
            } else {
                Err(SyncError::ChatScopeUnavailable)
            };
        }
        self.reservation = Some(Reservation {
            identity,
            observer,
            revoker,
            planned_revision: revision,
        });
        Ok(())
    }

    pub(crate) fn genesis_attachment_live(&self) -> bool {
        private_shape(self)
            && self.revision == 0
            && self.required
            && self.label.is_some()
            && self.reservation.as_ref().is_some_and(|source| {
                !source.observer.is_cancelled()
                    && source.planned_revision.is_some_and(|revision| revision > 0)
            })
    }

    pub(crate) fn genesis_label_data(&self) -> Option<&ChatScopeLabel> {
        self.label.as_ref()
    }

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
            reservation: self.reservation.as_ref().map(|source| {
                (
                    source.identity.clone(),
                    source.observer.clone(),
                    source.planned_revision,
                )
            }),
        })
    }
}
impl OriginalInitializationObservation {
    /// Compare an actual original private source to its newly decoded receiver-only genesis
    /// attachment. Its process-local revision is0, not the old source revision; only SAME
    /// reservation identity, saved labels, durable planned revision and original negatives can
    /// bridge this reconstruction. No publication runtime or newer owner is adopted.
    pub(crate) fn matches_genesis_attachment(&self, scope: &LocalChatScope) -> bool {
        scope.genesis_attachment_live()
            && self.required
            && self.label == scope.label
            && match (&self.reservation, &scope.reservation) {
                (Some((identity, negative, revision)), Some(current)) => {
                    !negative.is_cancelled()
                        && Arc::ptr_eq(identity, &current.identity)
                        && *revision == current.planned_revision
                }
                _ => false,
            }
    }

    /// Complete original host-planned scope labels for the private bootstrap DATA producer.
    /// The durable row revision must have been captured from the actual original reservation;
    /// the local process revision and scalar saved labels cannot supply it. This observation
    /// still needs the host's atomic admission through its actual save edge.
    pub(crate) fn planned_archive_context(
        &self,
        scope: &LocalChatScope,
    ) -> Result<([u8; 32], [u8; 32], u64), SyncError> {
        let domain = self.managed_archive_domain(scope)?;
        let revision = self
            .reservation
            .as_ref()
            .and_then(|(_, _, revision)| *revision)
            .filter(|revision| *revision != 0)
            .ok_or(SyncError::ChatScopeUnavailable)?;
        let label = self.label.as_ref().ok_or(SyncError::ChatScopeUnavailable)?;
        Ok((domain, label.nonce, revision))
    }
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
                (Some((identity, negative, revision)), Some(current)) => {
                    !negative.is_cancelled()
                        && Arc::ptr_eq(identity, &current.identity)
                        && *revision == current.planned_revision
                }
                _ => false,
            }
    }
}
