//! Read exact current cases from the selected keyed Case index. No call
//! here enters a signer, repairs a control, syncs a file or selects a head.
use super::attempt::JournalKey;
use super::observation::ReviewDecisionObservation;
use super::selection::{
    disk::{
        indexed::{IndexedDecisionLookup, IndexedDecisionSnapshot},
        HeldDecisionFile,
    },
    index::DecisionIndexQuery,
    HEAD_BYTES, MAX_RECORD_BYTES,
};
use super::*;
use crate::store::persistence::v2_review_hold::{NativeTextHeldCase, NativeTextHeldReviewDecision};
use catcoms_replication::chat::{
    cold_index::{MAX_INDEX_FRAME_BYTES, MAX_INDEX_HEIGHT},
    owner_bodies::ReviewStreamPosition,
    FounderGenesisAnchorData,
};
use catcoms_storage::VaultPhysicalWorkLease;

const MAX_CASES: usize = 8;
// One selected tip, at most eight authenticated Case paths and records,
// decoded record/signature scratch, result copies, paths and native File
// wrappers. This independent reservation remains until the page is dropped.
const READ_WORK: usize = (8 + 4 * MAX_CASES) * MAX_RECORD_BYTES
    + 5 * MAX_CASES * (1 + MAX_INDEX_HEIGHT as usize) * MAX_INDEX_FRAME_BYTES
    + 4 * (MAX_CASES * (1 + MAX_INDEX_HEIGHT as usize) + 8) * (governance::MAX_BASE_PATH + 256)
    + 2 * std::mem::size_of::<FounderReviewStatuses>()
    + 2 * std::mem::size_of::<IndexedStatusFiles>()
    + MAX_CASES
        * (std::mem::size_of::<IndexedDecisionLookup>()
            + (1 + MAX_INDEX_HEIGHT as usize) * std::mem::size_of::<HeldDecisionFile>())
    + 128 * 1024;

enum SelectedStatusFiles {
    Empty(HeldDecisionFile),
    // Paid read backing is allocated before native's final writers.
    Indexed(Box<IndexedStatusFiles>),
}

struct IndexedStatusFiles {
    snapshot: IndexedDecisionSnapshot,
    cases: Vec<IndexedDecisionLookup>,
}

/// Native selected identities are kept alongside the cached statuses.
/// Final publication only consults the revision; all file I/O belongs to
/// the original blocking founder read, before native's final writers.
pub(in crate::store) struct FounderReviewStatuses {
    states: [NativeTextHeldReviewDecision; MAX_CASES],
    len: usize,
    original: HeldDirectory,
    marker_path: PathBuf,
    marker: (File, SessionId),
    marker_bytes: Zeroizing<Vec<u8>>,
    physical: PhysicalMountNegative,
    selected: SelectedStatusFiles,
    observation: ReviewDecisionObservation,
    _lease: VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

impl FounderReviewStatuses {
    pub(in crate::store) fn states(&self) -> &[NativeTextHeldReviewDecision] {
        &self.states[..self.len]
    }

    pub(in crate::store) fn with_publication<T>(
        &self,
        publish: impl FnOnce() -> T,
    ) -> Result<T, AppError> {
        self.observation.with_publication(publish)
    }

    fn require(&mut self, store: &ServerStore) -> Result<(), AppError> {
        store.require_original_physical_mount(&self.physical)?;
        self.original.require()?;
        crate::store::persistence::v2_member_outbox::fs::require_exact_held(
            &self.marker_path,
            &mut self.marker,
            &self.marker_bytes,
        )?;
        require_settled(&self.original)?;
        match &mut self.selected {
            SelectedStatusFiles::Empty(head) => head.require()?,
            SelectedStatusFiles::Indexed(files) => {
                files.snapshot.require_current(&self.original)?;
                for case in &mut files.cases {
                    case.require_current()?;
                }
            }
        }
        self.original.require()
    }
}

impl ServerStore {
    /// Failure disables only the optional decision statuses. The caller
    /// keeps independently authenticated ID10 contents usable, and must
    /// still reject a cancelled/stale overall founder read.
    pub(in crate::store) fn read_founder_review_statuses<'a>(
        &self,
        pin: &FounderGenesisAnchorData,
        channel: u128,
        held: impl ExactSizeIterator<Item = &'a NativeTextHeldCase>,
        mut require: impl FnMut(Option<ReviewStreamPosition>) -> Result<(), AppError>,
    ) -> Result<FounderReviewStatuses, AppError> {
        let len = held.len();
        if len > MAX_CASES {
            return Err(denied());
        }
        let mounted = self
            .review_decision_mount
            .try_lock()
            .map_err(|_| denied())?;
        let mount = mounted.as_ref().ok_or_else(denied)?;
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let fee = policy.current.archive.pending.pool.try_reserve(READ_WORK)?;
        let lease = self._session.retain_for_physical_work();
        require(None)?;
        mount.require(self)?;
        if mount.warm.is_some() {
            return Err(denied());
        }
        require_settled(&mount.directory)?;
        let vault_key = Zeroizing::new(self.keys.db_key()?);
        let marker = mount.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let key = JournalKey::new(&vault_key, marker);
        let (states, selected) =
            read_cases(&mount.directory, &key, pin, channel, held, &mut require)?;
        require(None)?;
        mount.require(self)?;
        let mut result = FounderReviewStatuses {
            states,
            len,
            original: mount.directory.try_clone()?,
            marker_path: fixed(self, false, MARKER_NAME)?,
            marker: (
                mount.marker.0.try_clone().map_err(io)?,
                session_id(&mount.marker.0).map_err(io)?,
            ),
            marker_bytes: mount.marker.1.clone(),
            physical: self.capture_physical_mount_negative(),
            selected,
            observation: mount.publication.observe()?,
            _lease: lease,
            _fee: fee,
        };
        require(None)?;
        result.require(self)?;
        #[cfg(test)]
        if let Some(hook) = AFTER_REVIEW_STATUS_READ.with(std::cell::Cell::take) {
            hook();
        }
        Ok(result)
    }
}

fn require_settled(root: &HeldDirectory) -> Result<(), AppError> {
    // A pending signing attempt is not an undecided case. In particular,
    // a cold ambiguous Attempt must never be turned into a fresh UI action.
    control::require_control_names(root)?;
    selection::disk::indexed::require_quiet_selection(root)?;
    control::require_no_signing_controls(root)?;
    root.require()
}

fn read_cases<'a>(
    root: &HeldDirectory,
    key: &JournalKey,
    pin: &FounderGenesisAnchorData,
    channel: u128,
    held: impl ExactSizeIterator<Item = &'a NativeTextHeldCase>,
    require: &mut impl FnMut(Option<ReviewStreamPosition>) -> Result<(), AppError>,
) -> Result<
    (
        [NativeTextHeldReviewDecision; MAX_CASES],
        SelectedStatusFiles,
    ),
    AppError,
> {
    if held.len() > MAX_CASES {
        return Err(denied());
    }
    require_settled(root)?;
    let mut states = [NativeTextHeldReviewDecision::Unavailable; MAX_CASES];
    let mut head = HeldDecisionFile::read(root.child(HEAD_NAME), HEAD_BYTES)?.ok_or_else(denied)?;
    if head.bytes.as_slice() == key.initial_head() {
        // Authentic empty bytes alone do not erase unexpected outputs.
        InitialReviewHead::require_names(root, false)?;
        require(None)?;
        head.require()?;
        states[..held.len()].fill(NativeTextHeldReviewDecision::Undecided);
        return Ok((states, SelectedStatusFiles::Empty(head)));
    }
    // Legacy/unindexed heads and damaged/unsupported records refuse here;
    // no fallback fabricates an empty current case.
    let mut snapshot = IndexedDecisionSnapshot::read(root, key, pin)?;
    if snapshot.selected_data().1 != head.bytes.as_slice()
        || snapshot.selected_data().2.case_step_data().is_none()
    {
        return Err(denied());
    }
    require(Some(snapshot.selected_data().2.position_data()))?;
    let mut cases = Vec::with_capacity(held.len());
    for (slot, held) in states.iter_mut().zip(held) {
        let mut case = snapshot.lookup(root, key, pin, DecisionIndexQuery::Case(held.case))?;
        *slot = match &case.record {
            None => NativeTextHeldReviewDecision::Undecided,
            Some(record) => {
                let decision = &record.decision;
                let spec = decision.decision().spec();
                let target = spec.target.spec();
                let exact = decision.original_data();
                if decision.case_id() != held.case
                    || target.channel != channel
                    || target.action != held.signed.id()
                    || target.context.admission != held.admission
                    || target.subject != *held.signed.signer()
                    || exact.digest != held.original
                    || (exact.remove_header, exact.remove_epoch) != held.removal
                    || record.entry.case_step_data().is_none()
                {
                    return Err(denied());
                }
                require(Some(record.entry.position_data()))?;
                NativeTextHeldReviewDecision::Selected {
                    choice: spec.decision,
                    decision_body: decision.id(),
                    public_entry: record.entry.id(),
                }
            }
        };
        case.require_current()?;
        cases.push(case);
    }
    require(None)?;
    snapshot.require_current(root)?;
    for case in &mut cases {
        case.require_current()?;
    }
    head.require()?;
    require_settled(root)?;
    Ok((
        states,
        SelectedStatusFiles::Indexed(Box::new(IndexedStatusFiles { snapshot, cases })),
    ))
}

#[cfg(test)]
thread_local! {
    pub(in crate::store) static AFTER_REVIEW_STATUS_READ: std::cell::Cell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
mod tests;
