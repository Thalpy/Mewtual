//! Version3's distinct negative JOIN/index exclusion and its one actual payer.
//! This cannot be substituted for an old2 claim or a native/SDK credential.
//! Only the affine admission owner closes on Drop; policy/handler clones do not.
use super::{
    preparation::{Policy, PREPARATION_FEE},
    *,
};
use crate::store::{
    critical_namespaces::read_pool::CatalogReadReservation, CriticalCatalogReadPool,
};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

const CLOSED: u64 = u64::MAX;

struct Payment {
    pool: CriticalCatalogReadPool,
    // LAST: all metadata and every gate/claim reference precede the real refund.
    debit: CatalogReadReservation,
}

/// Narrow lifetime tail for finite files/buffers. In particular this does not
/// retain Handler.cold, which could otherwise cycle back to cold evidence.
pub(in crate::store::critical_namespaces::native_registry) struct PaymentTail {
    _payment: Arc<Payment>,
}

#[derive(Clone)]
pub(in crate::store) struct Handler {
    // Cold evidence has no Store back-reference. All original handles and its
    // finite physical-session lease retire before the actual payment tail.
    pub(super) cold: Arc<std::sync::Mutex<Option<super::inspection::Observation>>>,
    gate: Arc<Gate>,
    payment: Arc<Payment>,
}
impl Handler {
    pub(in crate::store::critical_namespaces::native_registry) fn payment_tail(
        &self,
    ) -> PaymentTail {
        PaymentTail {
            _payment: self.payment.clone(),
        }
    }
    /// Consume the SAME already-reserved full preparation invoice. This does
    /// not clone a debit or infer new capacity from extending its lifetime.
    pub(in crate::store::critical_namespaces::native_registry) fn from_reserved(
        policy: &Policy,
        debit: CatalogReadReservation,
    ) -> Result<Self, AppError> {
        debit.validate(&policy.pool, PREPARATION_FEE)?;
        let payment = Arc::new(Payment {
            pool: policy.pool.clone(),
            debit,
        });
        let gate = Arc::new(Gate {
            active: AtomicU64::new(0),
            serial: AtomicU64::new(0),
            _payment: payment.clone(),
        });
        Ok(Self {
            cold: Arc::new(std::sync::Mutex::new(None)),
            gate,
            payment,
        })
    }
    pub(in crate::store) fn require(&self) -> Result<(), AppError> {
        self.payment
            .debit
            .validate(&self.payment.pool, PREPARATION_FEE)
    }
    pub(in crate::store::critical_namespaces::native_registry) fn debit(
        &self,
    ) -> &CatalogReadReservation {
        &self.payment.debit
    }
    pub(in crate::store::critical_namespaces::native_registry) fn pool(
        &self,
    ) -> &CriticalCatalogReadPool {
        &self.payment.pool
    }
    /// Catalog-held checks are cached negative atomics only. Never acquire the
    /// payment mutex, read files or enter an SDK under this name.
    pub(in crate::store) fn require_idle_cached(&self) -> Result<(), AppError> {
        if self.gate.active.load(Ordering::SeqCst) == 0 {
            Ok(())
        } else {
            Err(refused())
        }
    }
    pub(in crate::store) fn close(&self) {
        self.gate.active.store(CLOSED, Ordering::SeqCst);
    }
    pub(in crate::store::critical_namespaces::native_registry) fn same(
        &self,
        other: &Self,
    ) -> bool {
        Arc::ptr_eq(&self.gate, &other.gate) && Arc::ptr_eq(&self.payment, &other.payment)
    }
    /// Only the owning producer calls this while holding the actual catalog
    /// mutex, after exact original Current/tuple/Pending proofs. The returned
    /// claim has no release-on-Drop, so lost callers cannot reopen JOIN.
    pub(in crate::store::critical_namespaces::native_registry) fn claim(
        &self,
    ) -> Result<Claim, AppError> {
        self.require_idle_cached()?;
        let serial = self
            .gate
            .serial
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_add(1).filter(|next| *next < CLOSED)
            });
        let serial = match serial {
            Ok(previous) => previous + 1,
            Err(_) => {
                let _ = self.gate.active.compare_exchange(
                    0,
                    CLOSED,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
                return Err(refused());
            }
        };
        self.gate
            .active
            .compare_exchange(0, serial, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| refused())?;
        Ok(Claim {
            original: self.gate.clone(),
            serial,
        })
    }
}

struct Gate {
    active: AtomicU64,
    serial: AtomicU64,
    _payment: Arc<Payment>,
}

pub(in crate::store::critical_namespaces::native_registry) struct Claim {
    original: Arc<Gate>,
    serial: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store::critical_namespaces::native_registry) enum Release {
    Idle,
    Closed,
    Foreign,
}

/// The completion outcome can be checked under the catalog guard, but retain
/// this WHOLE owner until after that guard drops. Even a final claim's payer
/// retirement must never acquire the account mutex under the writer.
#[must_use]
pub(in crate::store::critical_namespaces::native_registry) struct FinishedClaim {
    pub(in crate::store::critical_namespaces::native_registry) state: Release,
    _original: Arc<Gate>,
}
impl Claim {
    pub(in crate::store::critical_namespaces::native_registry) fn require_active(
        &self,
    ) -> Result<(), AppError> {
        if self.original.active.load(Ordering::SeqCst) != self.serial {
            return Err(refused());
        }
        Ok(())
    }
    /// Called only by the LAST completed-debt field after actual IO/buffer
    /// retirement, outside all writers. Foreign completion must fail closed;
    /// it cannot leave an unexpected successor admitted.
    pub(in crate::store::critical_namespaces::native_registry) fn finish_after_retirement(self) {
        let finished = self.finish();
        if finished.state == Release::Foreign {
            finished._original.active.store(CLOSED, Ordering::SeqCst);
        }
        drop(finished);
    }
    pub(in crate::store::critical_namespaces::native_registry) fn finish(self) -> FinishedClaim {
        let state = match self.original.active.compare_exchange(
            self.serial,
            0,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => Release::Idle,
            Err(CLOSED) => Release::Closed,
            Err(_) => Release::Foreign,
        };
        FinishedClaim {
            state,
            _original: self.original,
        }
    }
}

/// Arm the destination before disarming any outgoing upgrade wrapper. Drop
/// publishes closure first. The enclosing owner retains a handler/payment tail
/// through file and buffer retirement; retire this WHOLE owner outside writers,
/// because dropping its last Arc can also retire the actual payer.
pub(in crate::store::critical_namespaces::native_registry) struct AdmissionOwner {
    gate: Arc<Gate>,
}
impl AdmissionOwner {
    pub(in crate::store::critical_namespaces::native_registry) fn new(handler: &Handler) -> Self {
        Self {
            gate: handler.gate.clone(),
        }
    }
}
impl Drop for AdmissionOwner {
    fn drop(&mut self) {
        self.gate.active.store(CLOSED, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn paid() -> Handler {
        let policy = Policy::new(
            CriticalCatalogReadPool::new(PREPARATION_FEE).unwrap(),
            Policy::required_persistent_capacity(),
        )
        .unwrap();
        Handler::from_reserved(&policy, policy.reserve().unwrap()).unwrap()
    }
    #[test]
    fn lifecycle3_admission_requires_original_completion_and_handler_clones_do_not_close() {
        let handler = paid();
        let claim = handler.claim().unwrap();
        assert!(handler.claim().is_err());
        drop(handler.clone());
        let finished = claim.finish();
        assert_eq!(finished.state, Release::Idle);
        handler.require_idle_cached().unwrap();
        drop(finished);
        let next = handler.claim().unwrap();
        assert_eq!(next.finish().state, Release::Idle);
    }
    #[test]
    fn lifecycle3_admission_lost_claim_never_reopens_join() {
        let handler = paid();
        drop(handler.claim().unwrap());
        assert!(handler.require_idle_cached().is_err());
        assert!(handler.claim().is_err());
    }
    #[test]
    fn lifecycle3_admission_close_wins_over_original_completion() {
        let handler = paid();
        let claim = handler.claim().unwrap();
        handler.close();
        let finished = claim.finish();
        assert_eq!(finished.state, Release::Closed);
        assert!(handler.claim().is_err());
    }
    #[test]
    fn lifecycle3_admission_stale_completion_cannot_clear_a_successor() {
        let handler = paid();
        let first = handler.claim().unwrap();
        let serial = first.serial;
        drop(first.finish());
        let successor = handler.claim().unwrap();
        // Only this child test can construct a deliberately stale affine claim.
        let stale = Claim {
            original: handler.gate.clone(),
            serial,
        }
        .finish();
        assert_eq!(stale.state, Release::Foreign);
        assert!(handler.require_idle_cached().is_err());
        assert_eq!(successor.finish().state, Release::Idle);
    }
    #[test]
    fn lifecycle3_admission_finished_metadata_retires_before_actual_refund() {
        use std::sync::atomic::AtomicBool;
        let pool = CriticalCatalogReadPool::new(PREPARATION_FEE).unwrap();
        let policy = Policy::new(pool.clone(), Policy::required_persistent_capacity()).unwrap();
        let mut debit = policy.reserve().unwrap();
        let refunded = Arc::new(AtomicBool::new(false));
        debit.observe_drop_for_test(refunded.clone());
        let handler = Handler::from_reserved(&policy, debit).unwrap();
        let owner = AdmissionOwner::new(&handler);
        let finished = handler.claim().unwrap().finish();
        assert_eq!(finished.state, Release::Idle);
        drop(owner);
        assert!(handler.require_idle_cached().is_err());
        drop(handler);
        assert_eq!(pool.charged_for_test(), PREPARATION_FEE);
        assert!(!refunded.load(Ordering::SeqCst));
        drop(finished);
        assert_eq!(pool.charged_for_test(), 0);
        assert!(refunded.load(Ordering::SeqCst));
    }
    #[test]
    fn lifecycle3_admission_serial_exhaustion_is_terminal() {
        let handler = paid();
        handler.gate.serial.store(CLOSED - 1, Ordering::SeqCst);
        assert!(handler.claim().is_err());
        assert!(handler.require_idle_cached().is_err());
        assert_eq!(handler.gate.serial.load(Ordering::SeqCst), CLOSED - 1);
    }
}
