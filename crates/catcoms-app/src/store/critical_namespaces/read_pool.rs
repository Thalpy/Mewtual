//! Explicit logical catalog-read account. Clones share credits; fees keep their ORIGINAL
//! strong account alive until the actual data/copies/barriers have drained. No authority follows.
use super::refused;
use crate::AppError;
use std::sync::{Arc, Mutex};

/// Same enumerated Empty-only raw/frame/plaintext + metadata inventory as the v6 contract.
/// This is a logical reservation, not measured allocator/RSS/KDF/SDK or global native admission.
pub(in crate::store) const EMPTY_READ_FEE: usize = 8 * 1024;

struct State {
    capacity: usize,
    used: usize,
    unavailable: bool,
}
struct Inner {
    state: Mutex<State>,
    #[cfg(test)]
    entries: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    before_read: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

/// Caller-configured shared account for actual App catalog objects only. Supply clones of ONE
/// pool when opens/reads must compete for the same quota. This resource handle validates no
/// namespace or mount, and native/global/UI custody remains a separate integration requirement.
#[derive(Clone)]
pub struct CriticalCatalogReadPool {
    inner: Arc<Inner>,
}
impl std::fmt::Debug for CriticalCatalogReadPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CriticalCatalogReadPool")
            .finish_non_exhaustive()
    }
}
impl CriticalCatalogReadPool {
    /// Negative contention observation for actual blocked-I/O tests; no credit or guard.
    #[cfg(test)]
    pub(in crate::store) fn counter_available_for_test(&self) -> bool {
        self.inner.state.try_lock().is_ok()
    }

    #[cfg(test)]
    pub(in crate::store) fn read_entries_for_test(&self) -> usize {
        self.inner.entries.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(in crate::store) fn charged_for_test(&self) -> usize {
        self.inner.state.lock().expect("test catalog account").used
    }

    /// Set a trusted local logical hard capacity; never initialize this from decoded journal
    /// limits or untrusted IPC. Construction reserves no buffers and promises no physical RAM.
    pub fn new(capacity: usize) -> Result<Self, AppError> {
        if capacity == 0 || capacity > isize::MAX as usize {
            return Err(refused());
        }
        Ok(Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    capacity,
                    used: 0,
                    unavailable: false,
                }),
                #[cfg(test)]
                entries: std::sync::atomic::AtomicUsize::new(0),
                #[cfg(test)]
                before_read: Mutex::new(None),
            }),
        })
    }

    pub(in crate::store) fn try_reserve(
        &self,
        bytes: usize,
    ) -> Result<CatalogReadReservation, AppError> {
        if bytes == 0 {
            return Err(refused());
        }
        let accepted = match self.inner.state.lock() {
            Ok(mut state) => {
                let used = state
                    .used
                    .checked_add(bytes)
                    .filter(|used| !state.unavailable && *used <= state.capacity);
                if let Some(used) = used {
                    state.used = used;
                    true
                } else {
                    false
                }
            }
            Err(error) => {
                drop(error);
                false
            }
        };
        // Even the diagnostic String allocation occurs OUTSIDE the account lock.
        if !accepted {
            return Err(refused());
        }
        // No fee may be dropped while this mutex is held. Construct it AFTER unlocking; it
        // retains a strong reference, so closing a Store/caller cannot discard its account.
        Ok(CatalogReadReservation {
            original: self.inner.clone(),
            bytes,
        })
    }
}

/// Non-Clone actual debit. Declare it AFTER retained files/buffers in the owner so those drop
/// first. Supplying a pool handle is not supplying this prepaid fee; every read reacquires it.
pub(in crate::store) struct CatalogReadReservation {
    original: Arc<Inner>,
    bytes: usize,
}
impl CatalogReadReservation {
    pub(in crate::store) fn validate(
        &self,
        pool: &CriticalCatalogReadPool,
        required: usize,
    ) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.original, &pool.inner) || self.bytes < required {
            return Err(refused());
        }
        let accepted = match self.original.state.lock() {
            Ok(state) => !state.unavailable && state.used >= self.bytes,
            Err(error) => {
                drop(error);
                false
            }
        };
        if !accepted {
            return Err(refused());
        }
        Ok(())
    }

    /// Test-only actual buffer-entry seam; taking/releasing the hook lock precedes calling
    /// user test code. No account/hook mutex is held over callback, I/O, allocation or await.
    #[cfg(test)]
    pub(in crate::store) fn before_read_buffer(&self) {
        use std::sync::atomic::Ordering;
        let hook = self.original.before_read.lock().expect("test hook").take();
        if let Some(hook) = hook {
            hook();
        }
        self.original.entries.fetch_add(1, Ordering::SeqCst);
    }
}
impl Drop for CatalogReadReservation {
    fn drop(&mut self) {
        if let Ok(mut state) = self.original.state.lock() {
            match state.used.checked_sub(self.bytes) {
                Some(used) if !state.unavailable => state.used = used,
                // Underflow/previous accounting loss is permanent. Do not lower used or
                // guess a refund that could let a later reader overcommit this same pool.
                _ => state.unavailable = true,
            }
        }
        // Poison also remains unavailable: future lock attempts refuse. The strong Arc is
        // released only after the mutex guard above has gone, never recursively inside it.
    }
}

#[cfg(test)]
mod tests;
