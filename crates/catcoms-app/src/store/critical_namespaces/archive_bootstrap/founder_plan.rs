//! Original actual creator -> RAM-only pre-row plan. No decoder, supplied Server, scalar
//! reservation or second revocation sender can enter this path. Physical target/intent saves
//! remain the persistence sibling's responsibility under the borrowed original writer.
use super::*;
use crate::store::chat_admission::predecessor::archive_founder::ArchiveFounderPlan;
use crate::store::persistence::{PreparedFoundingIntent, SavedFoundingIntent};
use catcoms_rt::RequestCancellation;
#[path = "founder_intent.rs"]
mod intent;

/// A source borrow is constructible only here, from the direct `Server::found` result retained
/// by the actual one-shot creator. Keeping it borrowed through Root I/O prevents this creator's
/// node/plan from being replaced while the original admission and file writers are waiting.
/// It is negative validation, never a saved-source/Current/publication capability.
pub(in crate::store) struct ActualFounderSource<'source, T: MeshTransport, R: CryptoRngCore> {
    creator: &'source ArchiveFounderCreation<T, R>,
}
impl<T: MeshTransport, R: CryptoRngCore> ActualFounderSource<'_, T, R> {
    pub(in crate::store) fn validate_original(&self, store: &ServerStore) -> Result<(), AppError> {
        self.creator.original.validate_original(store)?;
        self.validate_source_only()
    }
    /// Actual source/negative checks WITHOUT reacquiring Root. The concrete physical writer
    /// uses this only while its borrowed admission/pins/canonical and original Root are held;
    /// that writer separately verifies the exact captured directory/marker/session itself.
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        if self.creator.original.observer().is_cancelled() {
            return Err(refused());
        }
        self.creator.group_data()?;
        let server = self.creator.server.as_ref().ok_or_else(refused)?;
        // This source is still private and never run. Fresh presence is checked above; reject
        // a member completion or an externally altered scope before creating any RAM plan.
        let fresh = server.sync.fresh_chat_admission().ok_or_else(refused)?;
        if fresh.key_package_reference().is_some() || fresh.authenticated_welcome_hash().is_some() {
            return Err(refused());
        }
        Ok(())
    }

    pub(in crate::store) fn validate_captured(
        &self,
        captured: &CapturedFounderInvocation,
    ) -> Result<(), AppError> {
        if !std::ptr::eq(captured, &self.creator.original) {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn captured(&self) -> &CapturedFounderInvocation {
        &self.creator.original
    }
    pub(in crate::store) fn group(&self) -> Result<&[u8], AppError> {
        self.creator.group_data()
    }
    pub(in crate::store) fn fresh_attempt(&self) -> Result<[u8; 32], AppError> {
        self.creator
            .server
            .as_ref()
            .and_then(|server| server.sync.fresh_chat_admission())
            .map(|fresh| fresh.attempt_id())
            .ok_or_else(refused)
    }
    pub(in crate::store) fn debit_id(&self) -> Result<codec::DebitId, AppError> {
        self.creator.debit_id.ok_or_else(refused)
    }
    pub(in crate::store) fn untouched_scope(&self) -> Result<(), AppError> {
        let server = self.creator.server.as_ref().ok_or_else(refused)?;
        if server.sync.chat_scope_label().is_some() || server.sync.chat_scope_revision() != 0 {
            return Err(refused());
        }
        Ok(())
    }
}

/// Same creator retained across original vacancy capture, nonce sampling, registration and
/// target/intent failures. Nothing runnable or scope-bearing escapes this private owner.
pub struct ArchiveFounderPlanning<T: MeshTransport, R: CryptoRngCore> {
    // Metadata/control observers contain no strong sender. The actual creator below remains
    // the SOLE owner of the Root creation life, and its Drop closes all these originals.
    plan: Option<ArchiveFounderPlan>,
    founding: intent::OriginalFoundingFrames,
    prepared_founding: Option<PreparedFoundingIntent>,
    saved_founding: Option<std::sync::Arc<SavedFoundingIntent>>,
    creator: ArchiveFounderCreation<T, R>,
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for ArchiveFounderPlanning<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveFounderPlanning")
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveFounderPlanningProgress {
    /// Only an original weak exclusion is installed. No disk intent/source, row or Current.
    RamPlanRegisteredUnexposed,
    /// The actual immutable target marker and original small intent passed their file/parent
    /// barriers. No private S0/source, scope row, runnable Server or Current follows from this.
    FoundingIntentSavedUnexposed,
}
/// Pre-plan refusal returns the SAME original creator, including any saved SDK ticket/debt.
/// Returning it is not an adopter and cannot reset its once-only counters or cached nonce.
pub struct ReturnedArchiveFounderCreation<T: MeshTransport, R: CryptoRngCore> {
    creator: ArchiveFounderCreation<T, R>,
    error: AppError,
}
impl<T: MeshTransport, R: CryptoRngCore> ReturnedArchiveFounderCreation<T, R> {
    pub fn into_parts(self) -> (ArchiveFounderCreation<T, R>, AppError) {
        (self.creator, self.error)
    }
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for ReturnedArchiveFounderCreation<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReturnedArchiveFounderCreation")
            .finish_non_exhaustive()
    }
}
impl<T: MeshTransport, R: CryptoRngCore> ArchiveFounderCreation<T, R> {
    /// Consume only the SAME completed creator. Early or stale calls preserve it in the error;
    /// a caller can finish its original debit/SDK work and retry without replacing inputs.
    pub fn into_ram_plan(
        self,
        store: &ServerStore,
    ) -> Result<ArchiveFounderPlanning<T, R>, ReturnedArchiveFounderCreation<T, R>> {
        let source = ActualFounderSource { creator: &self };
        let result = source
            .validate_original(store)
            .and_then(|()| source.untouched_scope());
        match result {
            Ok(()) => Ok(ArchiveFounderPlanning {
                plan: None,
                founding: intent::OriginalFoundingFrames::default(),
                prepared_founding: None,
                saved_founding: None,
                creator: self,
            }),
            Err(error) => Err(ReturnedArchiveFounderCreation {
                creator: self,
                error,
            }),
        }
    }
}
impl<T: MeshTransport, R: CryptoRngCore> ArchiveFounderPlanning<T, R> {
    pub fn close(&self) {
        self.creator.close();
    }
    pub fn observer(&self) -> RequestCancellation {
        self.creator.original.observer()
    }

    /// This is a private source step, not user-visible creation success. Its nonce stays inside
    /// this exact plan even if cancellation or a new old-head reservation wins after sampling.
    /// It never creates an Initializing row merely to reserve negative ownership.
    pub fn prepare_ram_plan(
        &mut self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<ArchiveFounderPlanningProgress, AppError> {
        // An exact completed intent can be reobserved, never reinserted as another transient
        // task or sampled again. The actual saved-source check remains mandatory on retry.
        if self.saved_founding.is_some() {
            return self.save_founding_intent(store, rng);
        }
        let source = ActualFounderSource {
            creator: &self.creator,
        };
        source.validate_original(store)?;
        source.untouched_scope()?;
        if self.plan.is_none() {
            self.plan = Some(store.capture_original_archive_plan(&source)?);
        }
        store.register_original_archive_plan(
            &source,
            self.plan.as_mut().ok_or_else(refused)?,
            rng,
        )?;
        source.validate_original(store)?;
        Ok(ArchiveFounderPlanningProgress::RamPlanRegisteredUnexposed)
    }
}

#[cfg(test)]
#[path = "founder_plan_tests.rs"]
mod tests;
