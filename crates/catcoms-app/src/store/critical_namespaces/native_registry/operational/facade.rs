//! Owning mounted native vault, without a raw Store/Current escape. UI lifecycle
//! observations are necessary negatives, never Tauri/window/row credentials.
use super::native_ui::{
    UiAdmissionOwner, UiCandidate, UiCapture, UiGarbage, UiPreparation, UiRetirement,
};
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64};

/// The affine mounted native backend. Install only through the native factory,
/// which authenticates its window/session and supplies the actual AppState
/// atomics. The trusted Rust constructor alone authenticates no native caller.
///
/// No clone, Deref, Store/Current getter or backend extractor exists. Workers
/// eventually receive private physical/operation custody, never this close owner.
pub struct NativeTextVault {
    // FIRST, outside physical work custody. Drop's synchronous fences below run
    // before this registry can destroy owners/fees and wake arbitrary waiters.
    ui: UiAdmissionOwner,
    index: mount::NativeIndexStore,
}
impl NativeTextVault {
    /// Consume an original, uninstalled native index owner. This may allocate
    /// and fail; call outside guards before installation. Failure closes and
    /// destroys the consumed owner, never hands its raw Store to a legacy path.
    /// These host atomics are trusted process inputs, not renderer arguments.
    pub fn from_trusted_host(
        index: mount::NativeIndexStore,
        generation: Arc<AtomicU64>,
        authorized: Arc<AtomicU64>,
        requested: Arc<AtomicBool>,
        budget: NativeUiInvocationBudget,
    ) -> Result<Self, AppError> {
        let ui = UiAdmissionOwner::new(generation, authorized, requested, budget)?;
        Ok(Self { ui, index })
    }

    /// Whole-vault admission only. Always fence Host first, original index
    /// second, before taking any registry, operation or physical writer lock.
    /// This is SC/atomic bookkeeping, with no notification, IO or destruction.
    /// UI lock and one-row Leave use their distinct, narrower retirement paths.
    pub fn close_admission(&self) {
        self.ui.close_first();
        self.index.close_index_admission();
    }

    /// Infallible original capture; native captures the generation before
    /// parsing and funds this invocation before borrowing AppState's vault.
    /// Finish this returned capsule outside EVERY native/Root guard.
    pub fn capture_ui(&self, generation: u64, credit: NativeUiInvocationMetadata) -> UiPreparation {
        self.ui.prepare_capture(generation, credit)
    }

    /// Canonical lifecycle observation on this SAME original vault. This does
    /// not check a Tauri window, row/membership, physical files or native commit.
    /// Keep the candidate in an outer slot, including on refusal, so losing
    /// owners/custody are destroyed after native registry guards have ended.
    pub fn admit_ui(&self, candidate: &mut UiCandidate) -> Result<UiCapture, AppError> {
        self.ui.admit(candidate)
    }

    /// Negative original-vault/generation check only; no raw RT command or key
    /// capability escapes. Actual primitives still need their concrete cuts.
    pub fn require_ui(&self, observation: &UiCapture) -> Result<(), AppError> {
        self.ui.require_capture(observation).map(|_| ())
    }

    /// Final original/generation/canonical-retirement negatives with no mutex,
    /// pool, IO or metadata upgrade. Perform require_ui outside native commit
    /// first, and keep this observation owned outside that entire guard scope.
    /// Window/row/source/primitive admission remains independently mandatory.
    pub fn ui_is_current_cached(&self, observation: &UiCapture) -> bool {
        self.ui.is_current_cached(observation)
    }

    /// Native publishes its lock generation/request atomics BEFORE calling.
    /// Move a Started batch out, then retire/wait outside every guard. Repeated
    /// waiters must coalesce in one bounded native-owned task; AlreadyRetiring
    /// is not a completed lock, and this is not physical/SDK/worker drainage.
    pub fn begin_ui_retirement(&self, through: u64) -> Result<UiRetirement, AppError> {
        self.ui.begin_retirement(through)
    }

    /// Return actual retired metadata for destruction outside guards. Closed
    /// unentered pins keep their original invoice independently of this GC.
    pub fn collect_retired_ui(&self) -> Result<UiGarbage, AppError> {
        self.ui.collect_drained()
    }

    /// Historical bounded DATA only, serialized with this original index's
    /// pending/outgoing-buffer debt. Neither method creates a live backend.
    pub fn row_count_data(&self) -> Result<usize, AppError> {
        self.index.row_count_data()
    }
    pub fn lookup_creation_data(
        &self,
        invocation: [u8; 32],
        request: [u8; 32],
    ) -> Result<Option<NativeCreationStatusData>, AppError> {
        self.index.lookup_creation_data(invocation, request)
    }
}
impl Drop for NativeTextVault {
    fn drop(&mut self) {
        self.close_admission();
    }
}
impl std::fmt::Debug for NativeTextVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextVault").finish_non_exhaustive()
    }
}
