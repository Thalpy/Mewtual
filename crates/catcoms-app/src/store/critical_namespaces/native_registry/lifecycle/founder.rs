//! Concrete founder-only index transitions. These are storage DATA producers;
//! the owning native operation must separately retain the original device,
//! network reservation and whole founder driver before installing a runtime.
use super::super::data::FounderBindingData;
use super::{
    data::{Backend, Birth, BirthKind, Change, Index, Row},
    metadata::{Outcome, Plan},
    *,
};
use crate::store::{NativeCreationStateData, NativeCreationStatusData};
use zeroize::Zeroizing;

pub(super) const MAX_NETWORK_FRAME: usize = 64 * 1024;

/// Generated identities and the already-sealed network reservation are frozen
/// once by the actual owning operation. The cell compares EVERY field on a
/// retry, not just the renderer's token/digest. No row ID is guessed in advance.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store::critical_namespaces::native_registry) struct Pending {
    pub(in crate::store::critical_namespaces::native_registry) identity: [u8; 32],
    pub(in crate::store::critical_namespaces::native_registry) birth: Birth,
    pub(in crate::store::critical_namespaces::native_registry) name: String,
    pub(in crate::store::critical_namespaces::native_registry) is_dm: bool,
    pub(in crate::store::critical_namespaces::native_registry) network_frame: Zeroizing<Vec<u8>>,
}

/// This binding is historical DATA from the SAME retained whole driver. It is
/// neither a Source nor an SDK grant. Native must still check the original
/// operation/session and actual completed driver before runtime installation.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store::critical_namespaces::native_registry) struct Activate {
    pub(in crate::store::critical_namespaces::native_registry) id: u64,
    pub(in crate::store::critical_namespaces::native_registry) identity: [u8; 32],
    pub(in crate::store::critical_namespaces::native_registry) birth: Birth,
    pub(in crate::store::critical_namespaces::native_registry) binding: FounderBindingData,
}

impl Pending {
    pub(super) fn require_bounded(&self) -> Result<(), AppError> {
        if self.name.len() > super::super::data::MAX_NAME
            || !(40..=MAX_NETWORK_FRAME).contains(&self.network_frame.len())
            || self.birth.kind != BirthKind::Founder
            || self.birth.join_context.is_some()
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) fn prepare(&self, selected: Index, parent: [u8; 32]) -> Result<Plan, AppError> {
        self.require_bounded()?;
        selected.validate_data()?;
        if let Some(row) = selected.lookup_data(self.birth.invocation, self.birth.request)? {
            if row.identity != self.identity
                || row.name != self.name
                || row.is_dm != self.is_dm
                || !matches!(&row.backend, Backend::Pending { birth, network_frame }
                    if birth == &self.birth && network_frame == &self.network_frame)
            {
                return Err(refused());
            }
            return Ok(Plan::NoChange(Outcome::Creation(status(row)?)));
        }
        let mut candidate = successor(&selected, parent)?;
        let id = selected.last_issued_id.checked_add(1).ok_or_else(refused)?;
        candidate.last_issued_id = id;
        candidate.rows.push(Row {
            id,
            identity: self.identity,
            name: self.name.clone(),
            is_dm: self.is_dm,
            metadata_revision: 0,
            backend: Backend::Pending {
                birth: self.birth.clone(),
                network_frame: self.network_frame.clone(),
            },
        });
        if selected.successor_data(&candidate, parent)? != Change::Birth {
            return Err(refused());
        }
        let result = Outcome::Creation(status(candidate.rows.last().ok_or_else(refused)?)?);
        Ok(Plan::Change {
            selected: Box::new(selected),
            candidate: Box::new(candidate),
            result,
        })
    }
}

impl Activate {
    pub(super) fn require_bounded(&self) -> Result<(), AppError> {
        if self.birth.kind != BirthKind::Founder
            || self.birth.join_context.is_some()
            || self.binding.group.is_empty()
            || self.binding.group.len() > 256
            || self.birth.signer != self.binding.signer
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) fn prepare(&self, selected: Index, parent: [u8; 32]) -> Result<Plan, AppError> {
        self.require_bounded()?;
        selected.validate_data()?;
        let row = selected
            .lookup_data(self.birth.invocation, self.birth.request)?
            .ok_or_else(refused)?;
        if row.id != self.id
            || row.identity != self.identity
            || row.backend.birth() != Some(&self.birth)
        {
            return Err(refused());
        }
        if let Backend::Founder { binding, .. } = &row.backend {
            if binding != &self.binding {
                return Err(refused());
            }
            return Ok(Plan::NoChange(Outcome::Creation(status(row)?)));
        }
        if !matches!(row.backend, Backend::Pending { .. }) {
            return Err(refused());
        }
        // Rebase on the latest selected index, preserving every neighboring
        // Birth/Rename/Refresh, not the snapshot used to save this Pending row.
        let position = selected
            .rows
            .iter()
            .position(|row| row.id == self.id)
            .ok_or_else(refused)?;
        let mut candidate = successor(&selected, parent)?;
        candidate.rows[position].backend = Backend::Founder {
            birth: self.birth.clone(),
            binding: self.binding.clone(),
        };
        if selected.successor_data(&candidate, parent)? != Change::Activate {
            return Err(refused());
        }
        let result = Outcome::Creation(status(&candidate.rows[position])?);
        Ok(Plan::Change {
            selected: Box::new(selected),
            candidate: Box::new(candidate),
            result,
        })
    }
}

fn successor(selected: &Index, parent: [u8; 32]) -> Result<Index, AppError> {
    let mut candidate = Index::decode_data(&selected.encode_data()?, selected.capability)?;
    candidate.generation = selected.generation.checked_add(1).ok_or_else(refused)?;
    candidate.parent = Some(parent);
    Ok(candidate)
}

/// Closed founder projection. Unsupported joined/legacy creation receipts must
/// not silently acquire a founder result or be interpreted as an absent token.
pub(super) fn status(row: &Row) -> Result<NativeCreationStatusData, AppError> {
    let state = match &row.backend {
        Backend::Pending { birth, .. } if birth.kind == BirthKind::Founder => {
            NativeCreationStateData::Pending
        }
        Backend::Founder { .. } => NativeCreationStateData::Founder,
        Backend::RetiredFounder { .. } => NativeCreationStateData::Retired,
        Backend::AbandonedFounder { .. } => NativeCreationStateData::Abandoned,
        _ => return Err(refused()),
    };
    Ok(NativeCreationStatusData {
        id: row.id,
        identity: row.identity,
        state,
    })
}
