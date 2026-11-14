//! Structural semantics only; real mounted IO and recovery live in cell tests.
use super::super::{
    capability::Capability,
    data::{Birth, BirthKind, LegacyOrigin, MigrationOrigin, Row},
};
use super::*;
use crate::store::critical_namespaces::archive_current_namespace_codec::CurrentLayoutData;

fn index() -> Index {
    let layout = CurrentLayoutData {
        vault: [1; 32],
        root: [2; 32],
    };
    Index {
        capability: Capability::for_current_data(layout, &layout.encode_data().unwrap()).unwrap(),
        origin: MigrationOrigin {
            generation: 7,
            frame: [3; 32],
        },
        generation: 2,
        parent: Some([4; 32]),
        last_issued_id: 2,
        rows: (1..=2)
            .map(|id| Row {
                id,
                identity: [id as u8; 32],
                name: format!("group{id}"),
                metadata_revision: 0,
                is_dm: id == 1,
                backend: Backend::Legacy {
                    origin: LegacyOrigin::Imported,
                    invite: format!("invite{id}"),
                },
            })
            .collect(),
    }
}
fn input() -> Request {
    Request {
        id: 1,
        identity: [1; 32],
        expected_revision: 0,
        invite: "refreshed".into(),
    }
}

#[test]
fn lifecycle3_refresh_preserves_identity_birth_dm_name_highwater_and_other_rows() {
    for admitted in [false, true] {
        let mut old = index();
        if admitted {
            old.rows[0].backend = Backend::Legacy {
                origin: LegacyOrigin::Admitted(Birth {
                    kind: BirthKind::LegacyJoin,
                    invocation: [8; 32],
                    request: [9; 32],
                    signer: [10; 32],
                    network: [11; 32],
                    join_context: Some([12; 32]),
                }),
                invite: "invite1".into(),
            };
        }
        let Plan::Change {
            selected,
            candidate,
            result,
        } = prepare(old, &input(), [6; 32]).unwrap()
        else {
            panic!("one change")
        };
        assert_eq!(
            selected.successor_data(&candidate, [6; 32]).unwrap(),
            Change::RefreshLegacyInvite
        );
        assert_eq!(candidate.rows[0].name, selected.rows[0].name);
        assert_eq!(candidate.rows[0].is_dm, selected.rows[0].is_dm);
        assert!(candidate.rows[0].backend.birth() == selected.rows[0].backend.birth());
        assert!(candidate.rows[1] == selected.rows[1]);
        assert!(candidate.origin == selected.origin);
        assert_eq!(candidate.last_issued_id, selected.last_issued_id);
        assert_eq!(candidate.generation, selected.generation + 1);
        assert_eq!(result.revision, 1);
    }
}

#[test]
fn lifecycle3_refresh_equal_invite_is_only_a_current_revision_observation() {
    let mut r = input();
    r.invite = "invite1".into();
    assert!(matches!(
        prepare(index(), &r, [6; 32]),
        Ok(Plan::NoChange(Metadata { revision: 0, .. }))
    ));
    r.expected_revision = 1;
    assert!(prepare(index(), &r, [6; 32]).is_err());
    r.expected_revision = 0;
    r.identity[0] ^= 1;
    assert!(prepare(index(), &r, [6; 32]).is_err());
    r = input();
    r.id = 3;
    assert!(prepare(index(), &r, [6; 32]).is_err());
}

#[test]
fn lifecycle3_refresh_refuses_terminal_pending_and_exhausted_rows() {
    let mut old = index();
    old.rows[0].backend = Backend::RetiredLegacy {
        origin: LegacyOrigin::Imported,
        invite: "invite1".into(),
        terminal: [9; 32],
    };
    assert!(prepare(old, &input(), [6; 32]).is_err());
    let mut old = index();
    old.rows[0].backend = Backend::Pending {
        birth: Birth {
            kind: BirthKind::Founder,
            invocation: [8; 32],
            request: [9; 32],
            signer: [10; 32],
            network: [11; 32],
            join_context: None,
        },
        network_frame: zeroize::Zeroizing::new(vec![0; 40]),
    };
    assert!(prepare(old, &input(), [6; 32]).is_err());
    let mut old = index();
    old.generation = u64::MAX;
    assert!(prepare(old, &input(), [6; 32]).is_err());
    let mut old = index();
    old.rows[0].metadata_revision = u64::MAX;
    let mut r = input();
    r.expected_revision = u64::MAX;
    assert!(prepare(old, &r, [6; 32]).is_err());
    r = input();
    r.invite = "x".repeat(super::super::super::data::MAX_INVITE + 1);
    assert!(prepare(index(), &r, [6; 32]).is_err());
}

#[test]
fn lifecycle3_refresh_maximum_payload_cannot_spend_reserved_terminal_capacity() {
    let mut old = index();
    // All active rows keep 36 bytes each for retirement. Fill the exact existing
    // reservation while leaving row1's invite empty for a rejected expansion.
    old.rows = (1u64..=100)
        .map(|id| Row {
            id,
            identity: *blake3::hash(&id.to_be_bytes()).as_bytes(),
            name: String::new(),
            is_dm: false,
            metadata_revision: 0,
            backend: Backend::Legacy {
                origin: LegacyOrigin::Imported,
                invite: String::new(),
            },
        })
        .collect();
    old.last_issued_id = 100;
    let mut remaining = super::super::codec::MAX_PLAIN - old.reserved_length_data().unwrap();
    for row in old.rows.iter_mut().skip(1) {
        let n = remaining.min(super::super::super::data::MAX_INVITE);
        let Backend::Legacy { invite, .. } = &mut row.backend else {
            unreachable!()
        };
        *invite = "x".repeat(n);
        remaining -= n;
    }
    assert_eq!(remaining, 0);
    assert_eq!(
        old.reserved_length_data().unwrap(),
        super::super::codec::MAX_PLAIN
    );
    assert!(old.encoded_length().unwrap() < super::super::codec::MAX_PLAIN);
    let mut r = input();
    r.identity = old.rows[0].identity;
    r.invite = "x".into();
    assert!(prepare(old, &r, [6; 32]).is_err());
    // The individual maximum still works in a table with available capacity.
    r = input();
    r.invite = "x".repeat(super::super::super::data::MAX_INVITE);
    assert!(matches!(
        prepare(index(), &r, [6; 32]),
        Ok(Plan::Change { .. })
    ));
}
