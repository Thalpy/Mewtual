//! Strengthen an existing Current admission with its actual canonical writer lifetime.
//!
//! A head CAS protects storage, but does not cancel an old Actor's queued transport work when
//! the writer changes within the SAME Current admission. This subordinate original observer
//! reaches the existing lower chat admission check. No row lookup or generic cancellation
//! channel may mint/reconstruct it. The canonical owner alone owns its revoking registration.
use super::*;
use crate::store::owner_evidence::canonical::{OwnerHeadWriteGuard, OwnerWriterRegistration};

impl ChatAdmissionBinding {
    /// Conjoin ONLY the original opaque canonical writer under already-held admission -> head
    /// custody. The exact Current scope stays unchanged; a new writer cannot be attached to an
    /// old queued binding. Dropping/replacing its original registration cancels all such work.
    pub(crate) fn with_owner_writer(
        &self,
        permit: &ChatAdmissionStoragePermit<'_>,
        head: &OwnerHeadWriteGuard<'_>,
        registration: &OwnerWriterRegistration,
    ) -> Result<Self, AppError> {
        permit.validate_original(permit.store(), self.key.group(), self.key.signer())?;
        if permit.mode() != ChatAdmissionStorageMode::Current
            || permit.domain() != self.domain
            || permit.nonce() != self.nonce
            || permit.revision() != self.revision
            || !Arc::ptr_eq(permit.owner_identity(), &self.owner_identity)
            || !self.is_live()
        {
            return Err(refused());
        }
        // This checks the actual already-held head guard, original Store/admission and source
        // writer. It neither reacquires a lock nor adopts whichever registration is current.
        registration.validate_original(head, permit, self)?;
        let identity = registration.identity();
        if let Some(original) = &self.owner_writer_identity {
            return if Arc::ptr_eq(original, &identity) {
                Ok(self.clone())
            } else {
                Err(refused())
            };
        }
        let writer = registration.publication_authority();
        if writer.is_cancelled() {
            return Err(refused());
        }
        let mut binding = self.clone();
        binding.publication =
            self.publication
                .clone()
                .with_authority(Arc::new(CanonicalWriterAuthority {
                    admission: self.publication.clone(),
                    writer,
                }));
        // Dummy allocation only: this must not keep the revoking writer/mount alive.
        binding.owner_writer_identity = Some(identity);
        Ok(binding)
    }

    pub(crate) fn owner_writer_identity(&self) -> Option<&Arc<()>> {
        self.owner_writer_identity.as_ref()
    }
}

struct CanonicalWriterAuthority {
    admission: RequestCancellation,
    writer: RequestCancellation,
}
impl std::fmt::Debug for CanonicalWriterAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanonicalWriterAuthority")
            .finish_non_exhaustive()
    }
}
#[async_trait::async_trait]
impl RequestAuthority for CanonicalWriterAuthority {
    fn authorized(&self) -> bool {
        !self.admission.is_cancelled() && !self.writer.is_cancelled()
    }

    async fn revoked(&self) {
        let mut admission = self.admission.clone();
        let mut writer = self.writer.clone();
        if self.authorized() {
            // Both original cancellation seams register before checking their revocation state.
            // Neither observer retains its revoking owner, and a closed owner fails closed.
            tokio::select! {
                _ = admission.cancelled() => {},
                _ = writer.cancelled() => {},
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    fn observer() -> (tokio::sync::watch::Sender<bool>, RequestCancellation) {
        let (owner, receiver) = tokio::sync::watch::channel(false);
        (owner, RequestCancellation::new(receiver, None))
    }

    #[tokio::test]
    async fn writer_drop_wakes_original_conjunction_without_retaining_either_owner() {
        let (admission_owner, admission) = observer();
        let (writer_owner, writer) = observer();
        let authority = CanonicalWriterAuthority { admission, writer };
        assert!(authority.authorized());
        let mut waiting = std::pin::pin!(authority.revoked());
        std::future::poll_fn(|cx| {
            assert!(waiting.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(writer_owner);
        assert!(!authority.authorized());
        std::future::poll_fn(|cx| {
            assert!(waiting.as_mut().poll(cx).is_ready());
            std::task::Poll::Ready(())
        })
        .await;
        assert!(!*admission_owner.borrow());
    }

    #[tokio::test]
    async fn original_admission_revocation_refuses_even_while_writer_is_live() {
        let (admission_owner, admission) = observer();
        let (writer_owner, writer) = observer();
        let authority = CanonicalWriterAuthority { admission, writer };
        admission_owner.send_replace(true);
        assert!(!authority.authorized());
        let mut already_revoked = std::pin::pin!(authority.revoked());
        std::future::poll_fn(|cx| {
            assert!(already_revoked.as_mut().poll(cx).is_ready());
            std::task::Poll::Ready(())
        })
        .await;
        assert!(!*writer_owner.borrow());
    }
}
