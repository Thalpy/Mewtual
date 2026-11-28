//! Read-only access to a covered public review position. Access to the
//! current founder's transport and membership is checked by its owner.
use super::attempt::JournalKey;
use super::selection::{
    disk::{
        indexed::{require_quiet_selection, IndexedDecisionSnapshot},
        HeldDecisionFile,
    },
    HEAD_BYTES, MAX_RECORD_BYTES,
};
use super::*;
use catcoms_replication::chat::{
    cold_index::{MAX_INDEX_FRAME_BYTES, MAX_INDEX_HEIGHT},
    owner_bodies::{
        ReviewStreamEntry, ReviewStreamPosition, ReviewStreamQuery, ReviewStreamRecord,
    },
    FounderGenesisAnchorData,
};

pub(in crate::store) enum ReviewDelivery {
    Header(Box<ReviewStreamEntry>),
    Approval(Zeroizing<Vec<u8>>),
}

// A tip and candidate image, one decoded record and result, a directed
// authenticated path, and temporary copies used by verification.
const READ_WORK: usize =
    8 * MAX_RECORD_BYTES + 5 * (1 + MAX_INDEX_HEIGHT as usize) * MAX_INDEX_FRAME_BYTES + 64 * 1024;

impl ServerStore {
    pub(in crate::store) fn read_public_review(
        &self,
        pin: &FounderGenesisAnchorData,
        query: ReviewStreamQuery,
        mut require: impl FnMut(Option<ReviewStreamPosition>) -> Result<(), AppError>,
        check_approval: impl FnOnce(&ReviewStreamRecord<'_>) -> Result<bool, AppError>,
    ) -> Result<Option<ReviewDelivery>, AppError> {
        if query.sequence_data() == 0 {
            return Err(denied());
        }
        let mounted = self
            .review_decision_mount
            .try_lock()
            .map_err(|_| denied())?;
        let mount = mounted.as_ref().ok_or_else(denied)?;
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let _volume = self._session.retain_for_physical_work();
        let _fee = policy.current.archive.pending.pool.try_reserve(READ_WORK)?;
        let root = &mount.directory;
        let vault_key = Zeroizing::new(self.keys.db_key()?);
        let marker = mount.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let key = JournalKey::new(&vault_key, marker);
        let mut recheck = |position| {
            require(position)?;
            mount.require(self)
        };
        recheck(None)?;
        require_quiet_selection(root)?;
        let mut head =
            HeldDecisionFile::read(root.child(HEAD_NAME), HEAD_BYTES)?.ok_or_else(denied)?;
        if head.bytes.as_slice() == key.initial_head() {
            recheck(None)?;
            head.require()?;
            require_quiet_selection(root)?;
            return Ok(None);
        }

        let mut snapshot = IndexedDecisionSnapshot::read(root, &key, pin)?;
        if snapshot.selected_data().1 != head.bytes.as_slice() {
            return Err(denied());
        }
        let reply =
            snapshot.read_public_record(root, (&key, pin), query, recheck, check_approval)?;
        head.require()?;
        Ok(reply)
    }
}
