//! Independent, process-local publication ordering for selected ID11 data.
//! A revision is a negative check, never a decision or signing permission.
use super::*;
use std::sync::{Arc, Mutex, Weak};

// Arc counters, the actual mutex/value, and conservative alignment padding.
pub(super) const REVISION_BYTES: usize = 2 * std::mem::size_of::<usize>()
    + std::mem::size_of::<Mutex<u64>>()
    + 2 * std::mem::align_of::<Mutex<u64>>();

pub(super) struct ReviewDecisionRevision(Arc<Mutex<u64>>);

pub(super) struct ReviewDecisionObservation {
    original: Weak<Mutex<u64>>,
    revision: u64,
}

impl ReviewDecisionRevision {
    pub(super) fn new() -> Self {
        Self(Arc::new(Mutex::new(0)))
    }

    pub(super) fn observe(&self) -> Result<ReviewDecisionObservation, AppError> {
        let revision = *self.0.try_lock().map_err(|_| denied())?;
        Ok(ReviewDecisionObservation {
            original: Arc::downgrade(&self.0),
            revision,
        })
    }

    /// Called under the original journal owner before any mutation or
    /// recovery. Never wait behind a final synchronous native resolver.
    pub(super) fn invalidate(&self) -> Result<(), AppError> {
        let mut revision = self.0.try_lock().map_err(|_| denied())?;
        *revision = revision.checked_add(1).ok_or_else(denied)?;
        Ok(())
    }
}

impl ReviewDecisionObservation {
    pub(super) fn with_publication<T>(&self, publish: impl FnOnce() -> T) -> Result<T, AppError> {
        let original = self.original.upgrade().ok_or_else(denied)?;
        let revision = original.try_lock().map_err(|_| denied())?;
        if *revision != self.revision {
            return Err(denied());
        }
        Ok(publish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn founder_decision_observation_refuses_mutation_replacement_and_concurrent_publication() {
        let owner = ReviewDecisionRevision::new();
        let before = owner.observe().unwrap();
        assert_eq!(before.with_publication(|| 3).unwrap(), 3);
        // A final resolver holds the exact revision. A writer must refuse,
        // rather than wait while it holds another journal/native lock.
        before
            .with_publication(|| assert!(owner.invalidate().is_err()))
            .unwrap();
        owner.invalidate().unwrap();
        assert!(before
            .with_publication(|| panic!("stale decision released"))
            .is_err());
        let after = owner.observe().unwrap();
        assert_eq!(after.with_publication(|| 4).unwrap(), 4);
        drop(owner);
        let replacement = ReviewDecisionRevision::new();
        assert!(after
            .with_publication(|| panic!("replacement adopted"))
            .is_err());
        assert!(replacement
            .observe()
            .unwrap()
            .with_publication(|| ())
            .is_ok());
    }
}
