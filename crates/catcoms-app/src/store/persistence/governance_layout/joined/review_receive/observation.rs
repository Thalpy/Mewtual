//! Keep renderer publication ordered with an admitted review update.
//! Only a checked read of the selected receipt can capture a lane fence.
use super::{
    manifest::{ReviewScope, MAX_SCOPES},
    *,
};
use catcoms_rt::RequestCancellation;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};

#[derive(Default)]
pub(super) struct ReviewPublicationFences {
    lanes: Mutex<BTreeMap<ReviewScope, Weak<Mutex<u64>>>>,
}

pub(in crate::store) struct ReviewReceiptObservation {
    lane: Arc<Mutex<u64>>,
    revision: u64,
    original: RequestCancellation,
}

fn gate_busy<T>(error: std::sync::TryLockError<T>) -> AppError {
    match error {
        std::sync::TryLockError::WouldBlock => AppError::ChatV2ArchiveBusy,
        std::sync::TryLockError::Poisoned(_) => denied(),
    }
}

impl ReviewPublicationFences {
    pub(super) fn capture(
        &self,
        scope: ReviewScope,
        original: RequestCancellation,
    ) -> Result<ReviewReceiptObservation, AppError> {
        if original.is_cancelled() {
            return Err(denied());
        }
        let mut lanes = self.lanes.try_lock().map_err(gate_busy)?;
        lanes.retain(|_, guard| guard.strong_count() != 0);
        let lane = match lanes.get(&scope).and_then(Weak::upgrade) {
            Some(lane) => lane,
            None => {
                if lanes.len() == MAX_SCOPES {
                    return Err(AppError::ChatV2ReceiveCapacity);
                }
                let lane = Arc::new(Mutex::new(0));
                lanes.insert(scope, Arc::downgrade(&lane));
                lane
            }
        };
        let revision = *lane.try_lock().map_err(gate_busy)?;
        Ok(ReviewReceiptObservation {
            lane,
            revision,
            original,
        })
    }

    /// Called under the original review mount lock before canonical admission.
    /// A different lane's original receipt observation stays current.
    pub(super) fn invalidate(&self, scope: ReviewScope) -> Result<(), AppError> {
        let lanes = self.lanes.try_lock().map_err(gate_busy)?;
        if let Some(lane) = lanes.get(&scope).and_then(Weak::upgrade) {
            let mut revision = lane.try_lock().map_err(gate_busy)?;
            *revision = revision.checked_add(1).ok_or_else(denied)?;
        }
        Ok(())
    }
}

impl ReviewReceiptObservation {
    /// The caller already holds the ordinary target observation and will
    /// enter the current projection view in this synchronous callback.
    pub(in crate::store) fn with_publication<T>(
        &self,
        publish: impl FnOnce() -> T,
    ) -> Result<T, AppError> {
        let lane = self.lane.try_lock().map_err(gate_busy)?;
        if *lane != self.revision || self.original.is_cancelled() {
            return Err(denied());
        }
        Ok(publish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_replication::chat::join_bindings::GenesisAnchorBindingData;

    #[test]
    fn reviewed_receipt_publication_orders_a_same_lane_write_and_releases_other_lanes() {
        let scope = |group| {
            ReviewScope::derive_data(
                &[1; 32],
                &GenesisAnchorBindingData {
                    group: vec![group; 16],
                    founder: [2; 32],
                    profile: [3; 32],
                    admission: [4; 32],
                    tenure: [5; 32],
                    header: [6; 32],
                    state: [7; 32],
                },
            )
            .unwrap()
        };
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        let original = RequestCancellation::new(receiver, None);
        let fences = ReviewPublicationFences::default();
        let left = fences.capture(scope(8), original.clone()).unwrap();
        let right = fences.capture(scope(9), original.clone()).unwrap();
        let writer = left
            .with_publication(|| fences.invalidate(scope(8)))
            .unwrap();
        assert!(matches!(writer, Err(AppError::ChatV2ArchiveBusy)));
        fences.invalidate(scope(8)).unwrap();
        assert!(left
            .with_publication(|| panic!("a stale view cannot release text"))
            .is_err());
        assert_eq!(right.with_publication(|| 9).unwrap(), 9);
        let current = fences.capture(scope(8), original).unwrap();
        assert_eq!(current.with_publication(|| 8).unwrap(), 8);
        cancel.send(true).unwrap();
        assert!(right
            .with_publication(|| panic!("a closed original cannot release text"))
            .is_err());
    }
}
