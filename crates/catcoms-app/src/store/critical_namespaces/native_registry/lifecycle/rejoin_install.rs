//! Build an Index4 selection beside the held Index3 predecessor. This is a
//! data builder for the exact rejoin writer, never an active native row.
use super::{
    data::{self, Backend, Index, Row},
    founder::successor,
    joined,
    preparation::decode_selected_data,
    *,
};
use crate::store::persistence::native_registry::lifecycle::upgrade::UpgradeInventory;
use crate::store::{
    NativeTextJoinedRegistrationData, NativeTextJoinedRegistryRowData, ServerStore,
};
use catcoms_sync::SelectedRejoinRemoval;
use zeroize::Zeroizing;

pub(in crate::store) struct PreparedNativeRejoinIndex {
    index: Index,
    plain: Zeroizing<Vec<u8>>,
    row: NativeTextJoinedRegistryRowData,
}

impl PreparedNativeRejoinIndex {
    /// Only an authenticated older row and actually selected Remove can
    /// produce this candidate. The physical caller independently covers the
    /// replacement's original SDK and Core history before calling.
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        inventory: &UpgradeInventory,
        previous: NativeTextJoinedRegistrationData,
        removal: SelectedRejoinRemoval,
        replacement: NativeTextJoinedRegistrationData,
    ) -> Result<Self, AppError> {
        if inventory.index_stage().is_some()
            || inventory.seed().is_some()
            || inventory.catalog_stage().is_some()
            || inventory.has_upgrade_outputs()
        {
            return Err(refused());
        }
        let (selected, _) = decode_selected_data(store, inventory)?;
        let old = joined::require_original(&selected, previous)?;
        let Backend::Joined { birth, binding } = &old.backend else {
            return Err(refused());
        };
        let parent = *blake3::hash(inventory.index()).as_bytes();
        let mut candidate = successor(&selected, parent)?;
        let proof = data::rejoin::RemovalData {
            row_id: old.id,
            row_identity: old.identity,
            own_admission: *removal.1.as_bytes(),
            header: *removal.0.as_bytes(),
            epoch: removal.2,
        };
        let old_row = candidate
            .rows
            .iter_mut()
            .find(|row| row.id == old.id)
            .ok_or_else(refused)?;
        old_row.backend = Backend::RetiredJoined {
            birth: birth.clone(),
            binding: binding.clone(),
            terminal: proof.header,
        };
        let id = selected.last_issued_id.checked_add(1).ok_or_else(refused)?;
        let names = joined::local_names(old);
        let (identity, backend) = joined::binding(replacement, names.display);
        candidate.rows.push(Row {
            id,
            identity,
            is_dm: old.is_dm,
            name: names.group,
            metadata_revision: 0,
            backend,
        });
        candidate.last_issued_id = id;
        data::rejoin::require_rejoin_successor(&selected, &candidate, parent, &proof)?;
        let row = joined::receipt(candidate.rows.last().ok_or_else(refused)?);
        let plain = candidate.encode_rejoin_data()?;
        inventory.require(store)?;
        Ok(Self {
            index: candidate,
            plain,
            row,
        })
    }

    pub(in crate::store) fn encoded(&self) -> &[u8] {
        &self.plain
    }

    pub(in crate::store) fn row_data(&self) -> NativeTextJoinedRegistryRowData {
        self.row
    }

    /// Decode the authenticated replacement using only Index4 and the same
    /// predecessor capability. Equal plaintext also retains every old row.
    pub(in crate::store) fn require_exact(&self, plain: &[u8]) -> Result<(), AppError> {
        if Index::decode_rejoin_data(plain, self.index.capability)? != self.index
            || plain != self.plain.as_slice()
        {
            return Err(refused());
        }
        Ok(())
    }
}
