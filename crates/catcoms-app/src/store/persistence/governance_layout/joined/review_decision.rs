//! Physical custody of the founder's selected decision journal.
//! A mounted namespace cannot authorize a decision or select a stream head.
use super::*;
use crate::store::chat_admission::PhysicalMountNegative;
use crate::store::persistence::v2_member_outbox::fs::{bounded_entries, HeldDirectory};

pub(super) const DIRECTORY_NAME: &str = "chat-v2-review-decisions";
pub(super) const MARKER_NAME: &str = "chat-critical-namespaces.review-decision.marker";

pub(in crate::store) struct SelectedReviewDecision {
    pub(super) marker: OpenedArchiveObject,
    pub(super) directory: HeldDirectory,
    mount: PhysicalMountNegative,
    _fee: CatalogReadReservation,
}

impl SelectedReviewDecision {
    pub(super) fn capture(
        store: &ServerStore,
        key: &[u8; 32],
        descriptor: &[u8],
    ) -> Result<Self, AppError> {
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let directory = fixed(store, false, DIRECTORY_NAME)?;
        let bytes = directory
            .as_os_str()
            .len()
            .checked_add(16 * 1024)
            .ok_or_else(denied)?;
        let fee = policy.current.archive.pending.pool.try_reserve(bytes)?;
        let expected =
            critical_namespaces::review_decision_namespace::marker_data(key, descriptor)?;
        let marker =
            read(&fixed(store, false, MARKER_NAME)?, expected.len())?.ok_or_else(denied)?;
        if marker.1.as_slice() != expected {
            return Err(denied());
        }
        let value = Self {
            marker,
            directory: HeldDirectory::open(directory)?,
            mount: store.capture_physical_mount_negative(),
            _fee: fee,
        };
        value.require(store)?;
        Ok(value)
    }

    pub(super) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        store.require_original_physical_mount(&self.mount)?;
        self.directory.require()?;
        // This slice mounts only an empty journal. A subsequent journal
        // interpreter must classify every owned record before this changes.
        if !bounded_entries(&self.directory, 0)?.is_empty() {
            return Err(denied());
        }
        let (file, bytes) = &self.marker;
        exact_held(
            &fixed(store, false, MARKER_NAME)?,
            file,
            session_id(file).map_err(io)?,
            bytes,
        )?;
        self.directory.require()
    }
}

pub(super) fn require_absent(store: &ServerStore) -> Result<(), AppError> {
    absent(&fixed(store, false, MARKER_NAME)?)?;
    absent(&fixed(store, false, DIRECTORY_NAME)?)
}
