//! Paid point read for an exact, physically selected approval receipt.
//! Reads preserve all pending state and borrow no authority to repair it.
use super::{
    approval,
    header::ReceiverKey,
    index::Indexes,
    journal::pinned_founder,
    manifest::{ManifestKey, ReviewScope},
    objects::{lane_directory_name, HeldLane, MAX_VIEW_FILES},
    observation::ReviewReceiptObservation,
    plan::{ApprovalSelection, ReviewLaneRead},
    write::{MANIFEST_STAGE, WAL_NAME},
    *,
};
use catcoms_replication::chat::{
    cold_index::MAX_INDEX_FRAME_BYTES, join_bindings::GenesisAnchorBindingData,
    owner_bodies::ReviewStreamRecord, ActionId,
};

const READ_WORK: usize =
    3 * MAX_VIEW_FILES * MAX_INDEX_FRAME_BYTES + 6 * approval::MAX_APPROVAL_FRAME + 128 * 1024;

pub(in crate::store) struct SelectedReceivedApproval {
    pub(in crate::store) raw: Zeroizing<Vec<u8>>,
    pub(in crate::store) observation: ReviewReceiptObservation,
    _fee: CatalogReadReservation,
}

impl SelectedReviewReceiver {
    pub(super) fn read_approval(
        &self,
        store: &ServerStore,
        binding: &GenesisAnchorBindingData,
        (channel, action): (u128, ActionId),
    ) -> Result<Option<SelectedReceivedApproval>, AppError> {
        self.require(store)?;
        let require_settled = || -> Result<(), AppError> {
            absent(&self.directory.child(WAL_NAME))?;
            absent(&self.directory.child(MANIFEST_STAGE))
        };
        require_settled()?;
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let fee = policy.current.archive.pending.pool.try_reserve(READ_WORK)?;
        let _lease = store._session.retain_for_physical_work();
        let scope = ReviewScope::derive_data(&store.chat_vault_domain()?, binding)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let marker = self.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let lane_key = ReceiverKey::new(&key, marker, scope.as_bytes());
        let indexes = Indexes::new(&key, scope);
        let mut manifest = self.read_manifest(store, &ManifestKey::new(&key, marker))?;
        let Some(selected) = manifest.data.selected(scope) else {
            absent(&self.directory.child(&lane_directory_name(scope)))?;
            manifest.require_current(&self.directory)?;
            require_settled()?;
            return Ok(None);
        };
        let mut lane = HeldLane::read(&self.directory, (scope, selected), &lane_key)?;
        if lane.head.is_frozen() {
            return Err(denied());
        }
        let row = lane
            .head
            .approvals
            .map(|root| indexes.lookup_approved(root, action, |id| lane.page(id)))
            .transpose()?
            .flatten();
        let Some(indexed) = row else {
            lane.require(&self.directory)?;
            manifest.require_current(&self.directory)?;
            require_settled()?;
            return Ok(None);
        };
        let ciphertext = lane.approval(&indexed.receipt)?;
        let pin = pinned_founder(binding);
        let raw = approval::open(&lane_key, &pin, &ciphertext)?;
        let record = ReviewStreamRecord::decode(&pin, &raw).map_err(|_| denied())?;
        if record.held_data().target_data().spec().channel != channel {
            return Err(denied());
        }
        let read = ReviewLaneRead {
            head: Some(lane.head),
            key: &lane_key,
            indexes: &indexes,
        };
        let ApprovalSelection::Selected { row: header, .. } = read.select_approval(
            &pin,
            record.entry_data(),
            |id| lane.page(id),
            |digest| lane.header(digest),
        )?
        else {
            return Err(denied());
        };
        if indexed != approval::row(header, &record, &ciphertext)? {
            return Err(denied());
        }
        lane.require(&self.directory)?;
        manifest.require_current(&self.directory)?;
        require_settled()?;
        self.require(store)?;
        let observation = self
            .publication
            .capture(scope, store.chat_mount.observer())?;
        Ok(Some(SelectedReceivedApproval {
            raw,
            observation,
            _fee: fee,
        }))
    }
}
