//! Registry row for a covered native Welcome. Shape alone admits no writes.
use super::{
    data::{Backend, Birth, BirthKind, Change, Index, JoinedBinding, Row},
    founder::successor,
    metadata::{Outcome, Plan},
    *,
};
use crate::store::NativeTextJoinedRegistrationData;

/// Stable local registry row for the original completed joined request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeTextJoinedRegistryRowData {
    pub id: u64,
    pub identity: [u8; 32],
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::store::critical_namespaces::native_registry) struct Register {
    pub(in crate::store::critical_namespaces::native_registry) name: String,
    pub(in crate::store::critical_namespaces::native_registry) original:
        NativeTextJoinedRegistrationData,
}

impl Register {
    pub(super) fn require_bounded(&self) -> Result<(), AppError> {
        if self.name.is_empty() || self.name.len() > super::super::data::MAX_NAME {
            return Err(refused());
        }
        Ok(())
    }

    pub(super) fn prepare(&self, selected: Index, parent: [u8; 32]) -> Result<Plan, AppError> {
        let original = self.original;
        if let Some(row) = selected.lookup_data(original.invocation, original.request)? {
            match_row(row, original)?;
            return Ok(Plan::NoChange(Outcome::JoinedRegistration(receipt(row))));
        }
        if has_native_row(&selected) {
            return Err(refused());
        }
        let mut candidate = successor(&selected, parent)?;
        let id = selected.last_issued_id.checked_add(1).ok_or_else(refused)?;
        candidate.last_issued_id = id;
        let (identity, backend) = binding(original);
        candidate.rows.push(Row {
            id,
            identity,
            name: self.name.clone(),
            is_dm: false,
            metadata_revision: 0,
            backend,
        });
        let result =
            Outcome::JoinedRegistration(receipt(candidate.rows.last().ok_or_else(refused)?));
        if selected.successor_data(&candidate, parent)? != Change::RegisterNativeJoined {
            return Err(refused());
        }
        Ok(Plan::Change {
            selected: Box::new(selected),
            candidate: Box::new(candidate),
            result,
        })
    }
}

fn binding(original: NativeTextJoinedRegistrationData) -> ([u8; 32], Backend) {
    let context = original.context;
    let selected = original.selected;
    let identity = blake3::derive_key("catcoms/native-joined-row/v2", &context.original_pending);
    let birth = Birth {
        kind: BirthKind::NativeJoined,
        invocation: original.invocation,
        request: original.request,
        signer: original.signer,
        network: original.transport.2,
        join_context: Some(context.original_pending),
    };
    let binding = JoinedBinding {
        vault: context.vault,
        layout: context.root,
        group: original.group.to_vec(),
        signer: original.signer,
        source: context.original_pending,
        profile: original.profile,
        pin: context.pin,
        join_context: context.original_pending,
        own_add: *selected.own_admission.as_bytes(),
        initial_governing_tip: *selected.governing_add.as_bytes(),
        initial_current_basis: selected.state,
        joined_epoch: selected.epoch,
    };
    (identity, Backend::Joined { birth, binding })
}

pub(super) fn receipt(row: &Row) -> NativeTextJoinedRegistryRowData {
    NativeTextJoinedRegistryRowData {
        id: row.id,
        identity: row.identity,
    }
}

fn native_rows(index: &Index) -> impl Iterator<Item = &Row> {
    index.rows.iter().filter(|row| {
        row.backend
            .birth()
            .is_some_and(|birth| birth.kind == BirthKind::NativeJoined)
    })
}

pub(super) fn has_native_row(index: &Index) -> bool {
    native_rows(index).next().is_some()
}

pub(super) fn require_original(
    index: &Index,
    original: NativeTextJoinedRegistrationData,
) -> Result<&Row, AppError> {
    let mut rows = native_rows(index);
    let row = rows.next().ok_or_else(refused)?;
    match_row(row, original)?;
    if rows.next().is_some() {
        return Err(refused());
    }
    Ok(row)
}

fn match_row(row: &Row, original: NativeTextJoinedRegistrationData) -> Result<(), AppError> {
    let (identity, backend) = binding(original);
    if row.identity != identity || row.backend != backend || row.is_dm {
        return Err(refused());
    }
    Ok(())
}
