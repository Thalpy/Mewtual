//! Permanent negative admission for the original native source collection.
//!
//! This gate grants no source, device, row or SDK authority. Whole-vault Host
//! closure already fences the UI-derived primitive cuts; this additional bit
//! prevents a prepared collection or a late completed source being installed
//! behind that closure. Actual source counters and whole runtimes must still
//! be retired/drained by their generic owner outside every native/Root guard.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct SourceAdmission {
    scope: Arc<Scope>,
}

pub(super) struct Scope {
    host: catcoms_rt::NativeChatUiMountObservation,
    closed: AtomicBool,
    issued: AtomicBool,
    // LAST: this independent payment follows negative observations even after
    // the physical vault and its larger index allocation have been destroyed.
    _metadata: CatalogReadReservation,
}

impl SourceAdmission {
    /// Call only during owning facade construction, before AppState install.
    /// Its independent fee follows every retained negative observation.
    pub(super) fn new(host: catcoms_rt::NativeChatUiMountObservation) -> Result<Self, AppError> {
        let metadata = CriticalCatalogReadPool::new(METADATA_BYTES)?.try_reserve(METADATA_BYTES)?;
        Ok(Self {
            scope: Arc::new(Scope {
                host,
                closed: AtomicBool::new(false),
                issued: AtomicBool::new(false),
                _metadata: metadata,
            }),
        })
    }

    /// SC-only: no lock, allocation, notification, destructor or IO. Never
    /// replace this scope during UI reauthentication or source collection GC.
    pub(super) fn close_first(&self) {
        self.scope.closed.store(true, Ordering::SeqCst);
    }

    pub(super) fn observe(&self) -> Arc<Scope> {
        self.scope.clone()
    }
}
impl Drop for SourceAdmission {
    fn drop(&mut self) {
        self.close_first();
    }
}

impl Scope {
    pub(super) fn is_open(&self) -> bool {
        !self.closed.load(Ordering::SeqCst) && self.host.is_open()
    }

    /// Exactly one owning source collection per facade. Retiring it is not
    /// permission to construct another collection over a still-mounted vault.
    /// A losing preparation must keep its own payment until outside guards.
    pub(super) fn claim_once(&self) -> Result<(), AppError> {
        if !self.is_open() {
            return Err(refused());
        }
        self.issued
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| refused())?;
        if !self.is_open() {
            return Err(refused());
        }
        Ok(())
    }

    /// Paid cached original-mount comparison only. This is never a UI-session
    /// admission or proof that a founder/row exists.
    #[cfg(test)]
    pub(super) fn matches(&self, host: &catcoms_rt::NativeChatUiMountObservation) -> bool {
        self.is_open() && self.host.same_original(host)
    }

    pub(super) fn close_first(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

const METADATA_BYTES: usize = 4096;
const _: () =
    assert!(std::mem::size_of::<Scope>() + 3 * std::mem::size_of::<usize>() <= METADATA_BYTES);

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::NativeChatUiBinding;
    use std::sync::atomic::AtomicU64;

    fn host() -> NativeChatUiBinding {
        NativeChatUiBinding::from_trusted_host(
            Arc::new(AtomicU64::new(7)),
            Arc::new(AtomicU64::new(7)),
            Arc::new(AtomicBool::new(false)),
            Arc::new(()),
        )
    }

    #[test]
    fn source_admission_collection_claim_is_once_only_and_not_a_ui_grant() {
        let binding = host();
        let admission = SourceAdmission::new(binding.observe_mount()).unwrap();
        let scope = admission.observe();
        scope.claim_once().unwrap();
        assert!(scope.claim_once().is_err());
        assert!(scope.matches(&binding.observe_mount()));
        assert!(!scope.matches(&host().observe_mount()));
    }

    #[test]
    fn source_admission_owner_drop_closes_retained_scope_without_closing_ui() {
        let binding = host();
        let admission = SourceAdmission::new(binding.observe_mount()).unwrap();
        let scope = admission.observe();
        drop(admission);
        assert!(binding.observe_mount().is_open());
        assert!(!scope.is_open());
        assert!(scope.claim_once().is_err());
    }

    #[test]
    fn source_admission_original_host_close_blocks_an_unclaimed_collection() {
        let binding = host();
        let admission = SourceAdmission::new(binding.observe_mount()).unwrap();
        let scope = admission.observe();
        binding.close();
        assert!(!scope.is_open());
        assert!(scope.claim_once().is_err());
        scope.close_first();
        assert!(!scope.matches(&host().observe_mount()));
    }
}
