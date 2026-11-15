//! Refresh only the current active Legacy row's stored invite. This stores
//! already-minted DATA; it neither mints an invite nor grants join authority.
use super::{
    data::{Backend, Change, Index},
    rename::{Metadata, Plan},
    *,
};

#[derive(Clone, PartialEq, Eq)]
pub(in crate::store::critical_namespaces::native_registry) struct Request {
    pub(in crate::store::critical_namespaces::native_registry) id: u64,
    pub(in crate::store::critical_namespaces::native_registry) identity: [u8; 32],
    pub(in crate::store::critical_namespaces::native_registry) expected_revision: u64,
    pub(in crate::store::critical_namespaces::native_registry) invite: String,
}

/// Stay inside the same paid metadata cell. The shared revision prevents a
/// delayed invite refresh from rebasing over a newer rename, and conversely.
pub(super) fn prepare(
    selected: Index,
    request: &Request,
    parent: [u8; 32],
) -> Result<Plan, AppError> {
    if request.invite.len() > super::super::data::MAX_INVITE {
        return Err(refused());
    }
    selected.validate_data()?;
    let position = selected
        .rows
        .iter()
        .position(|row| row.id == request.id)
        .ok_or_else(refused)?;
    let row = &selected.rows[position];
    if row.identity != request.identity || row.metadata_revision != request.expected_revision {
        return Err(refused());
    }
    let Backend::Legacy { invite, .. } = &row.backend else {
        return Err(refused());
    };
    if invite == &request.invite {
        return Ok(Plan::NoChange(Metadata {
            id: row.id,
            identity: row.identity,
            revision: row.metadata_revision,
        }));
    }
    let revision = row.metadata_revision.checked_add(1).ok_or_else(refused)?;
    let generation = selected.generation.checked_add(1).ok_or_else(refused)?;
    let mut candidate = Index::decode_data(&selected.encode_data()?, selected.capability)?;
    candidate.generation = generation;
    candidate.parent = Some(parent);
    let Backend::Legacy { invite, .. } = &mut candidate.rows[position].backend else {
        return Err(refused());
    };
    *invite = request.invite.clone();
    candidate.rows[position].metadata_revision = revision;
    // Includes the codec's mandatory completion-space calculation. Optional
    // metadata growth cannot consume an existing row's terminal allowance.
    if selected.successor_data(&candidate, parent)? != Change::RefreshLegacyInvite {
        return Err(refused());
    }
    Ok(Plan::Change {
        selected: Box::new(selected),
        candidate: Box::new(candidate),
        result: Metadata {
            id: request.id,
            identity: request.identity,
            revision,
        },
    })
}

#[cfg(test)]
mod tests;
