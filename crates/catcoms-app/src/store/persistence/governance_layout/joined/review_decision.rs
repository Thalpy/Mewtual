//! Physical custody of the founder's selected decision journal.
//! A mounted namespace cannot authorize a decision or select a stream head.
use super::*;
use crate::store::chat_admission::PhysicalMountNegative;
use crate::store::persistence::v2_member_outbox::fs::{
    bounded_entries, finish_exact_write, open_exact_write, HeldDirectory, WriteMode,
};

mod attempt;
mod control;
#[cfg(test)]
pub(in crate::store) use control::{AFTER_REVIEW_KEY_FOR_TEST, BEFORE_REVIEW_KEY_FOR_TEST};
mod delivery;
mod observation;
mod read;
mod selection;
#[cfg(test)]
pub(in crate::store) use control::{with_review_control_fault, ReviewControlFault};
pub(in crate::store) use control::{
    DecisionControl, PreparedIndexedReviewSource, PreparedReviewBase, ReviewSigningSpec,
};
pub(in crate::store) use delivery::ReviewDelivery;
pub(in crate::store) use read::FounderReviewStatuses;
#[cfg(test)]
pub(in crate::store) use read::AFTER_REVIEW_STATUS_READ;

pub(super) const DIRECTORY_NAME: &str = "chat-v2-review-decisions";
pub(super) const MARKER_NAME: &str = "chat-critical-namespaces.review-decision.marker";
pub(super) const HEAD_NAME: &str = "head.current";

/// The original catalog claim covers an authenticated empty head before
/// any signer may enter this journal. Absence is never a new baseline.
pub(super) struct InitialReviewHead {
    file: (File, SessionId),
    bytes: [u8; 32],
}

impl InitialReviewHead {
    pub(super) fn capture(
        directory: &HeldDirectory,
        key: &[u8; 32],
        marker: &[u8; 32],
        mode: WriteMode,
    ) -> Result<Self, AppError> {
        let bytes = attempt::JournalKey::new(key, marker).initial_head();
        Self::require_names(directory, matches!(mode, WriteMode::Create))?;
        let file = open_exact_write(&directory.child(HEAD_NAME), mode)?;
        Ok(Self { file, bytes })
    }

    pub(super) fn cover(
        &mut self,
        directory: &HeldDirectory,
        already_covered: bool,
    ) -> Result<(), AppError> {
        Self::require_names(directory, false)?;
        if already_covered {
            self.require(directory)?;
        }
        finish_exact_write(&directory.child(HEAD_NAME), &mut self.file, &self.bytes)?;
        directory.sync()?;
        self.require(directory)
    }

    pub(super) fn require_names(
        directory: &HeldDirectory,
        allow_absent: bool,
    ) -> Result<(), AppError> {
        let names = bounded_entries(directory, 1)?;
        match names.as_slice() {
            [] if allow_absent => Ok(()),
            [name] if name == HEAD_NAME => Ok(()),
            _ => Err(denied()),
        }
    }

    pub(super) fn require(&self, directory: &HeldDirectory) -> Result<(), AppError> {
        Self::require_names(directory, false)?;
        exact_held(
            &directory.child(HEAD_NAME),
            &self.file.0,
            self.file.1,
            &self.bytes,
        )?;
        directory.require()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_selection_refuses_a_replacement_initial_head_on_warm_retry() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("journal");
        std::fs::create_dir(&path).unwrap();
        let directory = HeldDirectory::open(path).unwrap();
        let mut initial =
            InitialReviewHead::capture(&directory, &[1; 32], &[2; 32], WriteMode::Create).unwrap();
        let head = directory.child(HEAD_NAME);
        finish_exact_write(&head, &mut initial.file, &initial.bytes[..16]).unwrap();
        let parked = temp.path().join("saved-head");
        std::fs::rename(&head, &parked).unwrap();
        std::fs::write(&head, initial.bytes).unwrap();
        assert!(initial.cover(&directory, false).is_err());
        assert_eq!(std::fs::read(&head).unwrap(), initial.bytes);
        std::fs::remove_file(&head).unwrap();
        std::fs::rename(&parked, &head).unwrap();
        initial.cover(&directory, false).unwrap();
    }
}

pub(in crate::store) struct SelectedReviewDecision {
    pub(super) marker: OpenedArchiveObject,
    pub(super) directory: HeldDirectory,
    mount: PhysicalMountNegative,
    warm: Option<control::DecisionControlData>,
    publication: observation::ReviewDecisionRevision,
    _fee: CatalogReadReservation,
}

impl SelectedReviewDecision {
    fn mount_bytes(store: &ServerStore) -> Result<usize, AppError> {
        let directory = fixed(store, false, DIRECTORY_NAME)?;
        directory
            .as_os_str()
            .len()
            .checked_add(16 * 1024)
            .and_then(|bytes| bytes.checked_add(observation::REVISION_BYTES))
            .ok_or_else(denied)
    }

    pub(super) fn reserve_mount(store: &ServerStore) -> Result<CatalogReadReservation, AppError> {
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        policy
            .current
            .archive
            .pending
            .pool
            .try_reserve_classified(Self::mount_bytes(store)?, || AppError::ChatV2ArchiveBusy)
    }

    pub(super) fn capture(
        store: &ServerStore,
        key: &[u8; 32],
        descriptor: &[u8],
    ) -> Result<Self, AppError> {
        Self::capture_prepaid(store, key, descriptor, Self::reserve_mount(store)?)
    }

    pub(super) fn capture_prepaid(
        store: &ServerStore,
        key: &[u8; 32],
        descriptor: &[u8],
        fee: CatalogReadReservation,
    ) -> Result<Self, AppError> {
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        fee.validate(
            &policy.current.archive.pending.pool,
            Self::mount_bytes(store)?,
        )?;
        let expected =
            critical_namespaces::review_decision_namespace::marker_data(key, descriptor)?;
        let marker =
            read(&fixed(store, false, MARKER_NAME)?, expected.len())?.ok_or_else(denied)?;
        if marker.1.as_slice() != expected {
            return Err(denied());
        }
        let value = Self {
            marker,
            directory: HeldDirectory::open(fixed(store, false, DIRECTORY_NAME)?)?,
            mount: store.capture_physical_mount_negative(),
            warm: None,
            publication: observation::ReviewDecisionRevision::new(),
            _fee: fee,
        };
        value.require(store)?;
        Ok(value)
    }

    pub(super) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        store.require_original_physical_mount(&self.mount)?;
        self.directory.require()?;
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

impl ServerStore {
    /// One exact journal reader or writer at a time. Failure to classify an
    /// occupied journal keeps that operation unavailable without touching
    /// the mounted parent's ordinary history or governing archives.
    pub(in crate::store) fn with_review_decision_control<T>(
        &self,
        use_control: impl FnOnce(&SelectedReviewDecision, &mut DecisionControl) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let mut journal = self
            .review_decision_mount
            .try_lock()
            .map_err(|_| denied())?;
        let journal = journal.as_mut().ok_or_else(denied)?;
        let mut control = DecisionControl::capture(self, journal, None)?;
        let result = use_control(journal, &mut control);
        journal.warm = control.retained_warm();
        result
    }
}

pub(super) fn require_absent(store: &ServerStore) -> Result<(), AppError> {
    absent(&fixed(store, false, MARKER_NAME)?)?;
    absent(&fixed(store, false, DIRECTORY_NAME)?)
}
