//! One local metadata CAS, not a native command credential or historical
//! invocation receipt. The owning paid cell keeps the complete plan private.
use super::{
    data::{Backend, Change, Index},
    *,
};

#[derive(PartialEq, Eq)]
pub(in crate::store::critical_namespaces::native_registry) struct Request {
    pub(in crate::store::critical_namespaces::native_registry) id: u64,
    pub(in crate::store::critical_namespaces::native_registry) identity: [u8; 32],
    pub(in crate::store::critical_namespaces::native_registry) expected_revision: u64,
    pub(in crate::store::critical_namespaces::native_registry) name: String,
}

/// Fixed current/covered metadata only. It cannot start an SDK or acknowledge
/// an earlier caller after restart merely because its desired name matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store::critical_namespaces::native_registry) struct Metadata {
    pub(in crate::store::critical_namespaces::native_registry) id: u64,
    pub(in crate::store::critical_namespaces::native_registry) identity: [u8; 32],
    pub(in crate::store::critical_namespaces::native_registry) revision: u64,
}
pub(super) enum Plan {
    NoChange(Metadata),
    Change {
        selected: Box<Index>,
        candidate: Box<Index>,
        result: Metadata,
    },
}

/// Called only inside one paid retained-cell operation. These owned decoded
/// indexes must never escape as independently reusable unpaid preparations.
/// `parent` is DATA; the actual writer still proves the original held frame.
pub(super) fn prepare(
    selected: Index,
    request: &Request,
    parent: [u8; 32],
) -> Result<Plan, AppError> {
    if request.name.len() > super::super::data::MAX_NAME {
        return Err(refused());
    }
    selected.validate_data()?;
    let position = selected
        .rows
        .iter()
        .position(|row| row.id == request.id)
        .ok_or_else(refused)?;
    let row = &selected.rows[position];
    if row.identity != request.identity
        || row.metadata_revision != request.expected_revision
        || !matches!(
            row.backend,
            Backend::Legacy { .. } | Backend::Founder { .. } | Backend::Joined { .. }
        )
    {
        return Err(refused());
    }
    if row.name == request.name {
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
    candidate.rows[position].name = request.name.clone();
    candidate.rows[position].metadata_revision = revision;
    if selected.successor_data(&candidate, parent)? != Change::Rename {
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
mod tests {
    use super::super::{
        capability::Capability,
        data::{LegacyOrigin, MigrationOrigin, Row},
    };
    use super::*;
    use crate::store::critical_namespaces::archive_current_namespace_codec::CurrentLayoutData;

    fn index() -> Index {
        // Structural DATA only; no authenticated file, native producer, Source
        // or membership is manufactured by these small semantic fixtures.
        let layout = CurrentLayoutData {
            vault: [1; 32],
            root: [2; 32],
        };
        Index {
            capability: Capability::for_current_data(layout, &layout.encode_data().unwrap())
                .unwrap(),
            origin: MigrationOrigin {
                generation: 1,
                frame: [3; 32],
            },
            generation: 1,
            parent: None,
            last_issued_id: 2,
            rows: (1..=2)
                .map(|id| Row {
                    id,
                    identity: [id as u8; 32],
                    name: format!("group{id}"),
                    metadata_revision: 0,
                    is_dm: false,
                    backend: Backend::Legacy {
                        origin: LegacyOrigin::Imported,
                        invite: format!("invite{id}"),
                    },
                })
                .collect(),
        }
    }
    fn request() -> Request {
        Request {
            id: 1,
            identity: [1; 32],
            expected_revision: 0,
            name: "local name".into(),
        }
    }

    #[test]
    fn lifecycle3_rename_plan_changes_one_revision_and_keeps_other_rows_and_origin() {
        let old = index();
        let other = old.rows[1].name.clone();
        let Plan::Change {
            selected,
            candidate,
            result,
        } = prepare(old, &request(), [4; 32]).unwrap()
        else {
            panic!("one rename")
        };
        assert_eq!(candidate.rows[0].name, "local name");
        assert_eq!(candidate.rows[0].metadata_revision, 1);
        assert_eq!(candidate.rows[1].name, other);
        assert_eq!(candidate.rows[1].metadata_revision, 0);
        assert!(candidate.origin == selected.origin);
        assert_eq!(candidate.last_issued_id, selected.last_issued_id);
        assert_eq!(result.revision, 1);
        assert!(matches!(
            selected.successor_data(&candidate, [4; 32]),
            Ok(Change::Rename)
        ));
    }

    #[test]
    fn lifecycle3_rename_same_name_requires_current_revision_and_identity() {
        let mut input = request();
        input.name = "group1".into();
        assert!(matches!(
            prepare(index(), &input, [4; 32]),
            Ok(Plan::NoChange(Metadata { revision: 0, .. }))
        ));
        input.expected_revision = 1;
        assert!(prepare(index(), &input, [4; 32]).is_err());
        input.expected_revision = 0;
        input.identity = [9; 32];
        assert!(prepare(index(), &input, [4; 32]).is_err());
    }

    #[test]
    fn lifecycle3_rename_rejects_missing_terminal_oversize_and_exhausted_state() {
        let mut input = request();
        input.id = 3;
        assert!(prepare(index(), &input, [4; 32]).is_err());
        input = request();
        input.name = "x".repeat(super::super::super::data::MAX_NAME + 1);
        assert!(prepare(index(), &input, [4; 32]).is_err());
        let mut old = index();
        old.generation = u64::MAX;
        old.parent = Some([5; 32]);
        assert!(prepare(old, &request(), [4; 32]).is_err());
        let mut old = index();
        old.generation = 2;
        old.parent = Some([5; 32]);
        old.rows[0].metadata_revision = u64::MAX;
        input = request();
        input.expected_revision = u64::MAX;
        assert!(prepare(old, &input, [4; 32]).is_err());
        let mut old = index();
        old.generation = 2;
        old.parent = Some([5; 32]);
        old.rows[0].backend = Backend::RetiredLegacy {
            origin: LegacyOrigin::Imported,
            invite: "invite1".into(),
            terminal: [8; 32],
        };
        assert!(prepare(old, &request(), [4; 32]).is_err());
    }
}
