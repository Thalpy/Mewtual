//! Negative DATA corruptions only. None of these scalar rows/owners becomes a real scope.
//! Actual original factory coverage is beside founder_plan_tests.rs. All tests authored UNRUN.
use super::*;

fn key(group: &[u8], signer: u8) -> ChatAdmissionKey {
    ChatAdmissionKey::new(group, [signer; 32]).unwrap()
}
fn row(phase: Phase) -> Record {
    Record {
        nonce: [1; 32],
        revision: 1,
        phase,
        origin: Origin::Legacy,
        provenance: [2; 32],
    }
}
fn runtime() -> Runtime {
    Runtime {
        managed: true,
        table: Some(BTreeMap::new()),
        ..Runtime::default()
    }
}
fn owner() -> AdmissionOwner {
    AdmissionOwner {
        nonce: [1; 32],
        revision: 1,
        life: revocation::Owner::new(),
    }
}

#[test]
fn every_phase_and_both_signer_endpoints_reserve_the_entire_group() {
    let bounds = ChatAdmissionGroupBounds::new(b"g").unwrap();
    for phase in [Phase::Initializing, Phase::Current, Phase::Retired] {
        for signer in [0, 1, 127, 255] {
            let mut runtime = runtime();
            runtime
                .table
                .as_mut()
                .unwrap()
                .insert(key(b"g", signer), row(phase));
            assert!(bounds.require_absent(&runtime).is_err());
            assert!(bounds
                .require_no_foreign(&runtime, &key(b"g", signer))
                .is_ok());
            assert!(bounds
                .require_no_foreign(&runtime, &key(b"g", signer ^ 1))
                .is_err());
        }
    }
}

#[test]
fn complete_group_boundaries_do_not_include_prefix_or_neighbor_groups() {
    let bounds = ChatAdmissionGroupBounds::new(b"g").unwrap();
    let mut runtime = runtime();
    for group in [b"f".as_slice(), b"g\0", b"gg", b"h"] {
        for signer in [0, 255] {
            runtime
                .table
                .as_mut()
                .unwrap()
                .insert(key(group, signer), row(Phase::Retired));
            runtime.owners.insert(key(group, signer), owner());
        }
    }
    assert!(bounds.require_absent(&runtime).is_ok());
    assert!(bounds.require_no_foreign(&runtime, &key(b"g", 255)).is_ok());
    assert!(bounds
        .require_no_foreign(&runtime, &key(b"gg", 255))
        .is_err());
    let capacities = (bounds.lower.group.capacity(), bounds.upper.group.capacity());
    for _ in 0..4 {
        assert!(bounds.require_absent(&runtime).is_ok());
    }
    assert_eq!(
        (bounds.lower.group.capacity(), bounds.upper.group.capacity()),
        capacities
    );
    assert!(ChatAdmissionGroupBounds::new(b"").is_err());
    assert!(ChatAdmissionGroupBounds::new(&[1; 257]).is_err());
}

#[test]
fn orphan_owner_cannot_hide_behind_missing_row_and_foreign_after_own_is_seen() {
    let bounds = ChatAdmissionGroupBounds::new(b"g").unwrap();
    let mut runtime = runtime();
    runtime.owners.insert(key(b"g", 255), owner());
    assert!(bounds.require_absent(&runtime).is_err());
    assert!(bounds.require_no_foreign(&runtime, &key(b"g", 0)).is_err());
    assert!(bounds.require_no_foreign(&runtime, &key(b"g", 255)).is_ok());
    runtime
        .table
        .as_mut()
        .unwrap()
        .insert(key(b"g", 0), row(Phase::Current));
    runtime
        .table
        .as_mut()
        .unwrap()
        .insert(key(b"g", 1), row(Phase::Retired));
    assert!(bounds.require_no_foreign(&runtime, &key(b"g", 0)).is_err());
}

#[test]
fn absent_requires_clean_loaded_inventory_but_no_foreign_can_inspect_same_dirty_table() {
    let bounds = ChatAdmissionGroupBounds::new(b"g").unwrap();
    let own = key(b"g", 7);
    let mut runtime = runtime();
    assert!(bounds.require_absent(&runtime).is_ok());
    runtime.dirty = true;
    assert!(bounds.require_absent(&runtime).is_err());
    assert!(
        bounds.require_no_foreign(&runtime, &own).is_ok(),
        "does not claim own row exists/is durable"
    );
    runtime
        .table
        .as_mut()
        .unwrap()
        .insert(own.clone(), row(Phase::Current));
    assert!(bounds.require_no_foreign(&runtime, &own).is_ok());
    runtime.table = None;
    assert!(bounds.require_absent(&runtime).is_err());
    assert!(bounds.require_no_foreign(&runtime, &own).is_err());
    runtime.table = Some(BTreeMap::new());
    runtime.managed = false;
    assert!(bounds.require_absent(&runtime).is_err());
    assert!(bounds.require_no_foreign(&runtime, &own).is_err());
}
