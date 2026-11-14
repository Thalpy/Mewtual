//! Owning mounted native vault, without a raw Store/Current escape. UI lifecycle
//! observations are necessary negatives, never Tauri/window/row credentials.
use super::native_ui::{
    UiAdmissionOwner, UiCandidate, UiCapture, UiGarbage, UiPreparation, UiRetirement,
};
use super::*;
pub(super) mod upgrade;
use std::sync::atomic::{AtomicBool, AtomicU64};
pub use upgrade::{NativeTextVaultUpgrade, NativeTextVaultUpgradeRefusal};

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
    // Negative only. Actual generic SourceOwners/runtimes are closed and
    // drained separately; this bit cannot stand in for their return.
    pub(super) sources: super::source_admission::SourceAdmission,
    pub(super) index: IndexOwner,
}
// Version2 remains a compatibility/testing owner. The real native factory
// explicitly consumes its upgrade continuation to obtain selected3; this enum
// never guesses a backend/version from decoded files or an opener error.
pub(super) enum IndexOwner {
    Version2(mount::NativeIndexStore),
    Version3(super::upgrade_source::owning::Store),
}
/// Infallible original capture, including refusals. Always retain this WHOLE
/// capsule until outside every guard: even Busy/Version2 holds its unique fee,
/// and a present work capsule can own the final original physical files.
pub struct NativeTextVaultAuthentication {
    work: Option<super::upgrade_source::owning::MetadataWork>,
    host: catcoms_rt::NativeChatUiMountObservation,
    expected: NativeUiInvocationBudget,
    // LAST: all original metadata/custody retires before this independent fee.
    credit: NativeUiInvocationMetadata,
}
/// Successful verification's original cached negatives only. No physical slot,
/// Store, close owner, Sender, cold lease, UI/Source/SDK grant or weak upgrade.
/// Keep the whole non-Clone observation outside native finalization guards.
pub struct NativeTextVaultAuthenticationCheck {
    host: catcoms_rt::NativeChatUiMountObservation,
    physical: super::upgrade_source::owning::AuthenticationNegative,
    _credit: NativeUiInvocationMetadata,
}
impl NativeTextVaultAuthentication {
    /// Blocking KDF verification outside EVERY guard. An independently funded
    /// invocation is checked against this original host's pool BEFORE either
    /// missing-work refusal or KDF. On success the WHOLE physical work is dropped
    /// before returning cached negatives, so continuity may capture its slot.
    pub fn verify(mut self, secret: &[u8]) -> Result<NativeTextVaultAuthenticationCheck, AppError> {
        self.expected.require_credit(&self.credit)?;
        if !self.host.is_open() {
            return Err(refused());
        }
        let work = self.work.take().ok_or_else(refused)?;
        if !work.is_live_cached() {
            return Err(refused());
        }
        let physical = work.authentication_negative()?;
        #[cfg(test)]
        AUTH_VERIFY_CALLS.with(|count| count.set(count.get() + 1));
        work.verify_native_passphrase(secret)?;
        let check = NativeTextVaultAuthenticationCheck {
            host: self.host,
            physical,
            _credit: self.credit,
        };
        // No guard surrounds this destructor. SlotAck releases only after any
        // last physical/Current destruction, using its original payment tail.
        drop(work);
        if !check.is_live_cached() {
            return Err(refused());
        }
        Ok(check)
    }
}
impl NativeTextVaultAuthenticationCheck {
    /// Original-mount correlation only. A UI lock may preserve this identity;
    /// it never proves a current UI generation or a newly authenticated secret.
    pub fn same_original(&self, other: &Self) -> bool {
        self.host.same_original(&other.host)
    }
    pub(super) fn matches_ui_mount(&self, host: &catcoms_rt::NativeChatUiMountObservation) -> bool {
        self.is_live_cached() && self.host.same_original(host)
    }
    /// Cached identity/negative check against the ACTUAL mounted facade. This
    /// allocates no identity and cannot substitute a replacement by directory.
    pub fn is_for_original_vault(&self, vault: &NativeTextVault) -> bool {
        self.matches_ui_mount(&vault.observe_ui_mount())
    }
    /// Cached original negatives only, not proof of UI authorization. This
    /// reads atomics and cannot drop a temporarily upgraded revoking owner.
    pub fn is_live_cached(&self) -> bool {
        self.host.is_open() && self.physical.is_live_cached()
    }
}
macro_rules! opaque_auth_debug {
    ($($ty:ty),+) => {$(impl std::fmt::Debug for $ty {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct(stringify!($ty)).finish_non_exhaustive()
        }
    })+};
}
opaque_auth_debug!(
    NativeTextVaultAuthentication,
    NativeTextVaultAuthenticationCheck
);

// Physical work already has its fixed original slot. This separately pins the
// actual capture/check/temporary metadata overlap to ONE unique invocation fee;
// it makes no claim about KDF working memory, native IPC or browser buffers.
const _: () = assert!(
    std::mem::size_of::<NativeTextVaultAuthentication>()
        + 2 * std::mem::size_of::<NativeTextVaultAuthenticationCheck>()
        + std::mem::size_of::<super::upgrade_source::owning::AuthenticationNegative>()
        + std::mem::size_of::<super::upgrade_source::owning::MetadataWork>()
        + 32 * std::mem::size_of::<usize>()
        <= super::native_ui::INVOCATION_BYTES
);

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
        let sources = super::source_admission::SourceAdmission::new(ui.observe_mount())?;
        Ok(Self {
            ui,
            sources,
            index: IndexOwner::Version2(index),
        })
    }

    /// Whole-vault admission only. Fence Host, source admission, then the
    /// original index before taking registry, operation or physical writers.
    /// This is SC/atomic bookkeeping, with no notification, IO or destruction.
    /// UI lock and one-row Leave use their distinct, narrower retirement paths.
    pub fn close_admission(&self) {
        self.ui.close_first();
        self.sources.close_first();
        match &self.index {
            IndexOwner::Version2(index) => index.close_index_admission(),
            IndexOwner::Version3(index) => index.close_admission(),
        }
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

    /// Infallible fixed capture under native's Store guard. Pay credit BEFORE
    /// that guard. Busy/Version2 is encoded as absent work. A closed Host may
    /// still have captured work while index closure follows; verify refuses it
    /// outside guards. Every refusal retains the SAME credit until then.
    /// It grants no permission; verify the caller's secret outside every guard.
    pub fn prepare_native_authentication(
        &self,
        credit: NativeUiInvocationMetadata,
    ) -> NativeTextVaultAuthentication {
        let work = match &self.index {
            IndexOwner::Version3(index) => index.capture_metadata_work(),
            IndexOwner::Version2(_) => None,
        };
        NativeTextVaultAuthentication {
            work,
            host: self.ui.observe_mount(),
            expected: self.ui.invocation_budget(),
            credit,
        }
    }

    pub(super) fn observe_ui_mount(&self) -> catcoms_rt::NativeChatUiMountObservation {
        self.ui.observe_mount()
    }
    /// Capture only the original dedicated UI slot. No index-WAL/Idle proof is
    /// substituted for local sidecar ownership. Native authenticates the caller.
    pub fn prepare_ui_state(
        &self,
        credit: super::ui_state::UiStateCredit,
    ) -> super::ui_state::UiStatePreparation {
        let work = match &self.index {
            IndexOwner::Version3(index) => index.capture_ui_work(),
            IndexOwner::Version2(_) => None,
        };
        super::ui_state::UiStatePreparation::new(self, work, credit, None)
    }

    /// Native continuity uses this original authenticated mount binding in
    /// addition to its own ACL and captured-generation admission. The owning
    /// capture remains infallible; wrong/closed targets refuse outside guards.
    pub fn prepare_ui_state_for_original(
        &self,
        credit: super::ui_state::UiStateCredit,
        target: Arc<NativeTextVaultAuthenticationCheck>,
    ) -> super::ui_state::UiStatePreparation {
        let work = match &self.index {
            IndexOwner::Version3(index) => index.capture_ui_work(),
            IndexOwner::Version2(_) => None,
        };
        super::ui_state::UiStatePreparation::new(self, work, credit, Some(target))
    }

    /// Historical bounded DATA only, serialized with this original index's
    /// pending/outgoing-buffer debt. Neither method creates a live backend.
    pub fn row_count_data(&self) -> Result<usize, AppError> {
        match &self.index {
            IndexOwner::Version2(index) => index.row_count_data(),
            IndexOwner::Version3(index) => index.native_row_count_data(),
        }
    }
    pub fn lookup_creation_data(
        &self,
        invocation: [u8; 32],
        request: [u8; 32],
    ) -> Result<Option<NativeCreationStatusData>, AppError> {
        match &self.index {
            IndexOwner::Version2(index) => index.lookup_creation_data(invocation, request),
            IndexOwner::Version3(index) => index.lookup_creation_data(invocation, request),
        }
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

#[cfg(test)]
thread_local! { static AUTH_VERIFY_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
