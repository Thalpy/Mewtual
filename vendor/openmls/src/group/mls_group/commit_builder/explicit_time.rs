//! The ordinary commit builder with an explicit clock for added KeyPackages.

use super::*;
use crate::group::validation_time::AddedKeyPackageValidationTime;
#[cfg(not(target_arch = "wasm32"))]
use std::time::SystemTime;
#[cfg(target_arch = "wasm32")]
use web_time::SystemTime;

impl<'a, G: BorrowMut<MlsGroup>> CommitBuilder<'a, LoadedPsks, G> {
    /// Build using a trusted caller time for the lifetimes of Add KeyPackages.
    ///
    /// This runs the same complete algorithm and proposal/capability/key checks
    /// as [`Self::build`]. Only the lifetime check for KeyPackages in the chosen
    /// Add proposals uses `now`; that check cannot be skipped by the ratchet
    /// tree lifetime policy or the upstream test-only lifetime skip flag. The
    /// existing proposal filter `f` has precisely its ordinary `build` meaning.
    ///
    /// The caller must derive `now` from its trusted local clock, never from a
    /// peer. This is not an all-SDK clock override: receive, join, other leaf
    /// validation paths and default lifetime generation retain their existing
    /// behavior. On wasm32 the time type is `web_time::SystemTime`, matching
    /// the existing [`crate::key_packages::Lifetime::validate_with_time`] API.
    ///
    /// This call performs SDK/cryptographic work. It grants no bounded operation
    /// reservation, Source/UI permission or publication authority; the caller
    /// must arrange those independently before entry and retain returned
    /// original work according to its actual persistence and retry contract.
    pub fn build_with_time<S: Signer>(
        self,
        rand: &impl OpenMlsRand,
        crypto: &impl OpenMlsCrypto,
        signer: &S,
        now: SystemTime,
        f: impl FnMut(&QueuedProposal) -> bool,
    ) -> Result<CommitBuilder<'a, Complete, G>, CreateCommitError> {
        self.build_internal(
            rand,
            crypto,
            signer,
            None::<NewSignerBundle<'_, S>>,
            AddedKeyPackageValidationTime::Explicit(now),
            crate::group::mls_group::text_add_execution::TextAddExecution::Legacy,
            f,
        )
    }
}
