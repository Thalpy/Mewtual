//! Per-actor admission for overlay jobs, and the ownership bundle a job must outlive its waiter
//! to hold.
//!
//! The bookkeeping deliberately stores no strong reference. An earlier shape kept the live token
//! in the record and relied on an explicit release, which a cancelled background waiter could
//! perform but an abandoned native preparation handle never would: dropping such a handle left
//! admission occupied for the lifetime of the actor. Holding only `Weak` handles removes the
//! transition entirely, so release is whatever happens when the last `Arc` drops, wherever it
//! lives, and no path has to remember anything.
use std::sync::{Arc, Weak};
use tokio::sync::OwnedSemaphorePermit;

/// The admission token and the shared preparation permit a detached worker, a retained result or
/// a native preparation handle must own for as long as it is alive. Dropping it releases both
/// together, so a job cannot hold capacity after its admission has gone or vice versa.
///
/// Design 5.5 originally gave this a third member, the job-owned reference hold; the revision
/// accepting its removal is in that section. A-001 made `AdmittedOverlayMedia` the single owner of
/// that hold, minted with the verified frame facts S3 rechecks and carried inside the capture and
/// the plan. A second `Option<CreativeHold>` here would be a competing owner, and a way to hold
/// pixels apart from the facts that justify holding them.
///
/// The three resources therefore do **not** always release together, and RT-001 is what happens
/// when code assumes they do: a refused plan's media hold dies with its capture, so this bundle
/// must be released in the worker at that moment rather than parked against a plan that can never
/// commit.
pub(crate) struct OverlayOwnership {
    // Held for their `Drop`, never read: the admission token's liveness IS the admission, and the
    // permit's existence IS the reserved slot. Reading either would be meaningless.
    #[allow(dead_code)]
    admission: Arc<()>,
    #[allow(dead_code)]
    permit: OwnedSemaphorePermit,
}

impl std::fmt::Debug for OverlayOwnership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OverlayOwnership { .. }")
    }
}

impl OverlayOwnership {
    pub(crate) fn new(admission: Arc<()>, permit: OwnedSemaphorePermit) -> Self {
        Self { admission, permit }
    }

    #[cfg(test)]
    pub(crate) fn admission_for_test(&self) -> Arc<()> {
        self.admission.clone()
    }

    #[cfg(test)]
    pub(crate) fn permit_for_test(&self) -> &OwnedSemaphorePermit {
        &self.permit
    }
}

/// At most one overlay job per actor is live, where live means at least one `Arc` clone of its
/// admission token still exists: in the tracked job, in a running detached worker whose waiter
/// was cancelled, in a retained result, or in a native preparation handle.
#[derive(Default)]
pub(crate) struct OverlayAdmission {
    /// Holds at most one entry, because `admit` pushes only when reaping left this empty.
    owners: Vec<Weak<()>>,
}

impl std::fmt::Debug for OverlayAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OverlayAdmission")
            .field("live", &self.owners.iter().any(|w| w.strong_count() != 0))
            .finish()
    }
}

impl OverlayAdmission {
    /// Reap owners whose job has finished, then report whether a new one may start. A dead entry
    /// disappears exactly when its last real owner does, with no message from that owner.
    pub(crate) fn can_admit(&mut self) -> bool {
        self.owners.retain(|owner| owner.strong_count() != 0);
        self.owners.is_empty()
    }

    /// Mint the token for one job. `None` means another job is still live somewhere.
    pub(crate) fn admit(&mut self) -> Option<Arc<()>> {
        if !self.can_admit() {
            return None;
        }
        let token = Arc::new(());
        self.owners.push(Arc::downgrade(&token));
        Some(token)
    }

    #[cfg(test)]
    pub(crate) fn tracked_for_test(&self) -> usize {
        self.owners.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> Arc<tokio::sync::Semaphore> {
        Arc::new(tokio::sync::Semaphore::new(4))
    }

    /// I-2. Admission depends on actual owners, never on an actor-side flag some path has to
    /// clear. Each case below ends an owner's life in a different way.
    #[test]
    fn overlay_admission_tracks_real_owners_through_every_way_a_job_can_end() {
        let mut admission = OverlayAdmission::default();
        let pool = pool();

        // An ordinary job: admitted, then released by dropping its ownership bundle.
        let token = admission.admit().expect("the first job is admitted");
        let ownership = OverlayOwnership::new(token, pool.clone().try_acquire_owned().unwrap());
        assert!(
            admission.admit().is_none(),
            "a second job was admitted while the first was live"
        );
        drop(ownership);
        assert!(admission.can_admit(), "a finished job kept admission");
        assert_eq!(admission.tracked_for_test(), 0, "dead owners are reaped");

        // A cancelled background waiter: the runtime drops its tracked job, but the blocking
        // closure still owns the bundle. Admission must stay unavailable until that closure ends.
        let token = admission.admit().expect("a new job is admitted");
        let still_running = OverlayOwnership::new(token, pool.clone().try_acquire_owned().unwrap());
        assert!(
            admission.admit().is_none(),
            "a cancelled waiter released admission while its worker was still running"
        );
        drop(still_running);
        assert!(admission.admit().is_some());

        // A retained result still owning a clone keeps admission unavailable, so ordinary
        // completion of the worker cannot clear it while the result is still parked.
        let token = admission.admit().expect("a new job is admitted");
        let worker = OverlayOwnership::new(token, pool.try_acquire_owned().unwrap());
        let parked_result = worker.admission_for_test();
        drop(worker);
        assert!(
            admission.admit().is_none(),
            "a retained result did not keep admission"
        );
        drop(parked_result);
        assert!(admission.admit().is_some());
    }

    /// The abandoned-native-handle case, which is why the record holds no strong reference: the
    /// handle is dropped without any second visit and without telling the actor anything.
    #[test]
    fn overlay_admission_recovers_from_an_abandoned_native_handle() {
        let mut admission = OverlayAdmission::default();
        let pool = pool();
        let token = admission.admit().expect("the preparation is admitted");
        let handle = OverlayOwnership::new(token, pool.clone().try_acquire_owned().unwrap());
        assert!(admission.admit().is_none());
        // Native cancels between visits, or simply drops the handle. No release is performed.
        drop(handle);
        assert!(
            admission.admit().is_some(),
            "an abandoned native handle occupied admission permanently"
        );
        // The shared permit came back with it, so the pool is not leaked either.
        assert_eq!(
            pool.available_permits(),
            4,
            "the abandoned handle leaked its preparation permit"
        );
    }

    /// The bundle releases its two pieces together, so a job cannot keep capacity after its
    /// admission has gone or vice versa. The job-owned reference hold is the capture's, not this
    /// bundle's; see the type comment for why A-001 makes that the only correct owner.
    #[test]
    fn overlay_ownership_releases_admission_and_capacity_together() {
        let mut admission = OverlayAdmission::default();
        let pool = pool();
        let token = admission.admit().unwrap();
        let weak = Arc::downgrade(&token);
        let ownership = OverlayOwnership::new(token, pool.clone().try_acquire_owned().unwrap());
        assert_eq!(pool.available_permits(), 3);
        assert_eq!(weak.strong_count(), 1);
        assert!(ownership.permit_for_test().num_permits() >= 1);
        assert!(Arc::ptr_eq(
            &ownership.admission_for_test(),
            &weak.upgrade().unwrap()
        ));
        drop(ownership);
        assert_eq!(pool.available_permits(), 4);
        assert_eq!(weak.strong_count(), 0);
        assert!(admission.can_admit());
    }
}
