//! Negative exclusion shared by the actual index owner and its Store's JOIN
//! handler. Idle never authorizes native work: all existing admission, original
//! source, file, membership and session checks remain mandatory independently.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

const CLOSED: u64 = u64::MAX;
pub(super) struct IndexInterlock {
    active: AtomicU64,
    serial: AtomicU64,
    // A claim is metadata only, but even that retained Arc must not survive its
    // exact paid lifetime. This creates no positive Store or native authority.
    _fee: Arc<payment::IndexPayment>,
}
impl IndexInterlock {
    pub(super) fn new(fee: Arc<payment::IndexPayment>) -> Self {
        Self {
            active: AtomicU64::new(0),
            serial: AtomicU64::new(0),
            _fee: fee,
        }
    }
    pub(super) fn idle(&self) -> bool {
        self.active.load(Ordering::Acquire) == 0
    }
    /// Called while holding the actual Store catalog mutex, after checking its
    /// original mounted.current, absence of Pending/Stage, and selected index.
    /// The retained index cell stores this claim before releasing that mutex.
    pub(super) fn claim(self: &Arc<Self>) -> Result<IndexClaim, AppError> {
        if !self.idle() {
            return Err(refused());
        }
        let serial = match self
            .serial
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |serial| {
                serial.checked_add(1).filter(|next| *next < CLOSED)
            }) {
            Ok(previous) => previous + 1,
            Err(_) => {
                let _ =
                    self.active
                        .compare_exchange(0, CLOSED, Ordering::AcqRel, Ordering::Acquire);
                return Err(refused());
            }
        };
        self.active
            .compare_exchange(0, serial, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refused())?;
        Ok(IndexClaim {
            original: self.clone(),
            serial,
        })
    }
    pub(super) fn close(&self) {
        self.active.store(CLOSED, Ordering::Release);
    }
}

/// Non-Clone and no release-on-Drop. Lost callers, unwinds and IO refusals must
/// not let JOIN overtake unresolved index debt. Whole-mount close is separate.
pub(super) struct IndexClaim {
    original: Arc<IndexInterlock>,
    serial: u64,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ClaimRelease {
    Idle,
    Closed,
}

/// Move this whole negative owner when the opaque mount changes wrappers. Its
/// first-field destruction closes admission BEFORE files, buffers or mount
/// retirement, without making any Handler clone's Drop close another owner.
pub(super) struct AdmissionOwner {
    gate: Arc<IndexInterlock>,
}
impl AdmissionOwner {
    pub(super) fn new(gate: Arc<IndexInterlock>) -> Self {
        Self { gate }
    }
}
impl Drop for AdmissionOwner {
    fn drop(&mut self) {
        self.gate.close();
    }
}
impl IndexClaim {
    /// Consume only after the same candidate's exact covering cleanup barriers
    /// or a proved no-effect disposal. A stale completion cannot clear another
    /// claim, and completion after close cannot reopen this original interlock.
    pub(super) fn finish(self) -> Result<ClaimRelease, AppError> {
        match self.original.active.compare_exchange(
            self.serial,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(ClaimRelease::Idle),
            Err(CLOSED) => Ok(ClaimRelease::Closed),
            Err(_) => Err(refused()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn paid_gate() -> Arc<IndexInterlock> {
        let pool = CriticalCatalogReadPool::new(OPERATIONAL_FEE).unwrap();
        Arc::new(IndexInterlock::new(Arc::new(payment::IndexPayment::new(
            pool.try_reserve(OPERATIONAL_FEE).unwrap(),
        ))))
    }

    #[test]
    fn native_index2_interlock_retains_debt_until_original_completion() {
        let gate = paid_gate();
        let claim = gate.claim().unwrap();
        assert!(!gate.idle());
        assert!(gate.claim().is_err());
        assert_eq!(claim.finish().unwrap(), ClaimRelease::Idle);
        assert!(gate.idle());
        assert!(gate.claim().is_ok());
    }
    #[test]
    fn native_index2_interlock_abandoned_claim_never_publishes_idle() {
        let gate = paid_gate();
        drop(gate.claim().unwrap());
        assert!(!gate.idle());
        assert!(gate.claim().is_err());
    }
    #[test]
    fn native_index2_interlock_closed_original_cannot_be_reopened_by_completion() {
        let gate = paid_gate();
        let claim = gate.claim().unwrap();
        gate.close();
        assert_eq!(claim.finish().unwrap(), ClaimRelease::Closed);
        assert!(!gate.idle());
        assert!(gate.claim().is_err());
    }
    #[test]
    fn native_index2_interlock_stale_claim_cannot_clear_successor_or_another_mount() {
        let gate = paid_gate();
        let claim = gate.claim().unwrap();
        let serial = claim.serial;
        claim.finish().unwrap();
        let successor = gate.claim().unwrap();
        // Deliberate test-only stale completion. No production constructor can
        // reconstruct or clone the already-consumed affine claim.
        assert!(IndexClaim {
            original: gate.clone(),
            serial
        }
        .finish()
        .is_err());
        assert!(!gate.idle());
        let other = paid_gate();
        let other_claim = other.claim().unwrap();
        successor.finish().unwrap();
        assert!(gate.idle());
        assert!(!other.idle());
        other_claim.finish().unwrap();
    }
    #[test]
    fn native_index2_interlock_serial_exhaustion_is_terminal_without_wrap() {
        let gate = paid_gate();
        gate.serial.store(CLOSED - 1, Ordering::Release);
        assert!(gate.claim().is_err());
        assert!(!gate.idle());
        assert_eq!(gate.serial.load(Ordering::Acquire), CLOSED - 1);
    }
    #[test]
    fn native_index2_interlock_concurrent_claims_have_exactly_one_original() {
        let gate = paid_gate();
        let start = Arc::new(std::sync::Barrier::new(3));
        let claims = std::thread::scope(|scope| {
            let a = scope.spawn({
                let gate = gate.clone();
                let start = start.clone();
                move || {
                    start.wait();
                    gate.claim().ok()
                }
            });
            let b = scope.spawn({
                let gate = gate.clone();
                let start = start.clone();
                move || {
                    start.wait();
                    gate.claim().ok()
                }
            });
            start.wait();
            [a.join().unwrap(), b.join().unwrap()]
        });
        assert_eq!(claims.iter().filter(|claim| claim.is_some()).count(), 1);
        claims
            .into_iter()
            .flatten()
            .next()
            .unwrap()
            .finish()
            .unwrap();
        assert!(gate.idle());
    }
}
