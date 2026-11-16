//! The saved-intent-only host reservation for the actual private source consumer. This is
//! never a Current/publication binding. Its original creator alone retains the Root sender.
use super::*;
use catcoms_sync::local_chat_scope::{ManagedChatScopeReservation, ManagedChatScopeRevocation};

pub(in crate::store) struct ArchiveFounderReservation<'source> {
    original: &'source ArchiveFounderPlan,
}
impl ArchiveFounderPlan {
    /// Only concrete saved physical custody can reach this adapter. It cannot be constructed
    /// from a decoded intent, registration status or matching nonce, and the transient slot
    /// must already have been replaced by this EXACT saved result before private attachment.
    pub(in crate::store) fn reservation_for_saved_source<'source>(
        &'source self,
        captured: &CapturedFounderInvocation,
        saved: &Arc<SavedFoundingIntent>,
    ) -> Result<ArchiveFounderReservation<'source>, AppError> {
        self.validate_captured(captured)?;
        let original = self.saved_foundation.as_ref().ok_or_else(refused)?;
        if self.registered || !self.nonce_sampled || !Arc::ptr_eq(original, saved) {
            return Err(refused());
        }
        saved.retains_original(captured, &self.facts, self.debit)?;
        Ok(ArchiveFounderReservation { original: self })
    }
}
impl ManagedChatScopeReservation for ArchiveFounderReservation<'_> {
    fn group(&self) -> &[u8] {
        &self.original.facts.group
    }
    fn signer(&self) -> &[u8; 32] {
        &self.original.facts.signer
    }
    fn domain(&self) -> [u8; 32] {
        self.original.facts.scope_domain
    }
    fn nonce(&self) -> [u8; 32] {
        self.original.facts.nonce
    }
    fn planned_revision_data(&self) -> Option<u64> {
        Some(self.original.facts.revision)
    }
    fn owner_identity(&self) -> Arc<()> {
        self.original.identity.clone()
    }
    fn reservation_observer(&self) -> RequestCancellation {
        self.original.observer.clone()
    }
    fn revocation(&self) -> Arc<dyn ManagedChatScopeRevocation> {
        Arc::new(self.original.revoker.clone())
    }
}
