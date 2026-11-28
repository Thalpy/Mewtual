//! Closed physical handshake for the companion catalog owner. This does
//! not detect mode from file presence and never creates a missing Head.
use super::*;
use crate::store::persistence::v2_shared_history::disk::indexed::{
    IndexedHeadOwner, IndexedHistoryLayout,
};

pub(in crate::store) const INDEXED_MARKER_NAME: &str =
    "chat-critical-namespaces.indexed-shared-history.marker";

pub(super) struct IndexedArchiveMount {
    layout: IndexedHistoryLayout,
    marker: OpenedArchiveObject,
    head: IndexedHeadOwner,
}
impl IndexedArchiveMount {
    pub(super) fn matches_original(&self, other: &Self) -> Result<bool, AppError> {
        Ok(self.layout == other.layout
            && self.head.matches_original(&other.head)?
            && self.marker.1 == other.marker.1
            && session_id(&self.marker.0).map_err(io)?
                == session_id(&other.marker.0).map_err(io)?)
    }

    pub(super) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        self.head.require()?;
        exact_held(
            &fixed(store, false, INDEXED_MARKER_NAME)?,
            &self.marker.0,
            session_id(&self.marker.0).map_err(io)?,
            &self.marker.1,
        )
    }
}

impl SelectedSharedHistory {
    /// Negative overlap check for the declaration/recovery's actual files.
    /// This neither reopens names nor adopts a caller's decoded binding;
    /// the physical caller still brackets it with its original proof.
    pub(in crate::store::persistence::governance_layout::joined) fn require_original_indexed_outputs(
        &self,
        marker: &File,
        head: &File,
        head_frame: &[u8],
    ) -> Result<(), AppError> {
        let indexed = self.indexed.as_ref().ok_or_else(denied)?;
        if session_id(marker).map_err(io)? != session_id(&indexed.marker.0).map_err(io)?
            || marker.metadata().map_err(io)?.len() != indexed.marker.1.len() as u64
        {
            return Err(denied());
        }
        let mut file = marker.try_clone().map_err(io)?;
        std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(0)).map_err(io)?;
        let mut bytes = [0u8; 32];
        std::io::Read::read_exact(&mut file, &mut bytes).map_err(io)?;
        if indexed.marker.1.as_slice() != bytes {
            return Err(denied());
        }
        self.directory.require()?;
        indexed.head.require_original_file(head, head_frame)
    }

    /// The caller is the authenticated ID13 catalog/declaration owner,
    /// before exposing a native row. `descriptor` remains the exact ID8
    /// entry; `companion` is the separately parsed exact ID13 entry. A
    /// caller DTO or the mere presence of new files cannot select this.
    /// Old capture() intentionally remains v1 and refuses indexed names.
    pub(in crate::store::persistence::governance_layout::joined) fn capture_indexed(
        store: &ServerStore,
        descriptor: &[u8],
        companion: &[u8],
        layout: CurrentLayoutData,
        vault_key: &[u8; 32],
    ) -> Result<Self, AppError> {
        let mut mounted = Self::capture(store, descriptor, layout, vault_key)?;
        let binding: [u8; 32] = companion.try_into().map_err(|_| denied())?;
        let marker = read(&fixed(store, false, INDEXED_MARKER_NAME)?, 32)?.ok_or_else(denied)?;
        let expected =
            crate::store::critical_namespaces::indexed_shared_history_namespace::marker_data(
                vault_key, companion,
            )?;
        if marker.1.as_slice() != expected {
            return Err(denied());
        }
        let key = member_outbox_key(store)?;
        let head =
            IndexedHeadOwner::capture(&mounted.directory, &key, IndexedHistoryLayout(binding))?;
        mounted.indexed = Some(IndexedArchiveMount {
            layout: IndexedHistoryLayout(binding),
            marker,
            head,
        });
        mounted.require(store)?;
        Ok(mounted)
    }

    pub(super) fn open_disk(
        &self,
        scope: SharedHistoryScope,
        pool: CriticalCatalogReadPool,
        key: &[u8; 32],
    ) -> Result<SharedHistoryDisk, AppError> {
        let directory = self.directory.try_clone()?;
        if let Some(indexed) = &self.indexed {
            SharedHistoryDisk::from_indexed_held(
                directory,
                scope,
                SharedHistoryDiskQuota::native(),
                pool,
                key,
                indexed.head.clone(),
            )
        } else {
            SharedHistoryDisk::from_held(
                directory,
                scope,
                SharedHistoryDiskQuota::native(),
                pool,
                key,
            )
        }
    }
}
