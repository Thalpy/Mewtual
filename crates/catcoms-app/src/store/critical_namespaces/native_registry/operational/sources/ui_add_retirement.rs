//! Bound Add cleanup of the original runtime cells before waiting for UI keys.
catcoms_rt::redacted_debug! {
    [T: MeshTransport, R: CryptoRngCore] NativeTextUiAddRetirement<T, R>;
}

use super::super::native_ui::RetirementDrain;
use super::*;
use std::sync::{TryLockError, Weak};

/// Retained alongside the original native UI retirement job. This captures
/// the original collection without keeping an issuer, runtime or vault alive.
pub struct NativeTextUiAddRetirement<T: MeshTransport, R: CryptoRngCore> {
    book: Weak<SourceBook<T, R>>,
    _control: Arc<retirement::Control>,
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextSources<T, R> {
    /// Atomic/Weak-only under a short canonical owner loan. Retain the result
    /// until outside all native and Root writers, including on refusal.
    pub fn capture_ui_add_retirement(&self) -> NativeTextUiAddRetirement<T, R> {
        NativeTextUiAddRetirement {
            book: Arc::downgrade(&self.entries),
            _control: self.retirement.clone(),
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextUiAddRetirement<T, R> {
    /// One bounded pass after the canonical UI batch closes. This may finish
    /// only already admitted work. The original Source remains available.
    /// After a lost caller, retry the same capture instead of a new mount.
    pub fn try_retire(&self, closed: &RetirementDrain) -> Result<bool, AppError> {
        if !closed.all_closed() {
            return Ok(false);
        }
        let Some(_pass) = self._control.try_begin_ui_add_pass() else {
            return Ok(false);
        };
        let Some(book) = self.book.upgrade() else {
            return Ok(closed.is_drained_cached());
        };
        let mut cells: [Option<Arc<RuntimeCell<T, R>>>; MAX_SOURCES] =
            std::array::from_fn(|_| None);
        {
            let entries = match book.try_lock() {
                Ok(entries) => entries,
                Err(TryLockError::WouldBlock) => return Ok(false),
                Err(TryLockError::Poisoned(_)) => return Err(refused()),
            };
            if entries.len() > cells.len() {
                return Err(refused());
            }
            for (target, original) in cells.iter_mut().zip(entries.iter()) {
                *target = Some(original.runtime.clone());
            }
        }
        drop(book);
        let counters = closed.add_cleanup_counters();
        let mut idle = true;
        for cell in cells.into_iter().flatten() {
            let (body, recorded) = {
                let mut state = match cell.state.try_lock() {
                    Ok(state) => state,
                    Err(TryLockError::WouldBlock) => {
                        idle = false;
                        continue;
                    }
                    Err(TryLockError::Poisoned(_)) => return Err(refused()),
                };
                if state.poisoned {
                    return Err(refused());
                }
                if state.retiring {
                    // Permanent Source retirement owns this cell and its Ends.
                    idle &= closed.is_drained_cached();
                    continue;
                }
                if state.running {
                    idle = false;
                    continue;
                }
                let body = state.body.take().ok_or_else(refused)?;
                state.running = true;
                (body, !state.interrupted)
            };
            let mut loan = RuntimeLoan {
                body: Some(body),
                cell,
                recorded,
            };
            let body = loan.body.as_mut().ok_or_else(refused)?;
            if let Some(driver) = body.driver.as_mut() {
                driver.retire_add_for_closed_ui(&counters)?;
            }
            if let Some(founder) = body.recovered_founder.as_mut() {
                founder
                    .retire_add_for_closed_ui(&counters, body.rng.as_mut().ok_or_else(refused)?)?;
            }
        }
        Ok(idle)
    }
}

pub(super) fn invoice<T: MeshTransport, R: CryptoRngCore>() -> usize {
    4 * std::mem::size_of::<NativeTextUiAddRetirement<T, R>>()
        + 2 * std::mem::size_of::<[Option<Arc<RuntimeCell<T, R>>>; MAX_SOURCES]>()
        + 1024
}
