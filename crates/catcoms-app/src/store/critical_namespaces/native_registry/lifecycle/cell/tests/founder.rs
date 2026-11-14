//! Real selected3 founder storage, never native/SDK authorization. Every fault
//! drives the real retained cell and actual cold opener, not a synthetic result.
use super::super::super::data::{Backend, Birth, BirthKind};
use super::super::super::founder::{Activate, Pending};
use super::*;
use crate::store::critical_namespaces::native_registry::data::FounderBindingData;
use crate::store::NativeCreationStateData;

fn pending(n: u8) -> Pending {
    Pending {
        identity: [n; 32],
        birth: Birth {
            kind: BirthKind::Founder,
            invocation: [n + 1; 32],
            request: [n + 2; 32],
            signer: [n + 3; 32],
            network: [n + 4; 32],
            join_context: None,
        },
        name: format!("founder {n}"),
        is_dm: false,
        network_frame: Zeroizing::new(vec![n; 40]),
    }
}
fn activation(owner: &owning::Store, input: &Pending) -> Activate {
    let index = selected(owner);
    let row = index
        .lookup_data(input.birth.invocation, input.birth.request)
        .unwrap()
        .unwrap();
    Activate {
        id: row.id,
        identity: row.identity,
        birth: input.birth.clone(),
        binding: FounderBindingData {
            vault: index.capability.original.vault,
            layout: index.capability.original.root,
            group: vec![41; 32],
            signer: input.birth.signer,
            source: [42; 32],
            profile: [43; 32],
            pin: [44; 32],
        },
    }
}
fn born(owner: &mut owning::Store, input: &Pending, rng: &mut ChaCha20Rng) {
    let (_, debt) = owner.birth_founder_data(input, rng).unwrap();
    assert!(debt.is_some());
    drop(debt);
}

#[test]
fn lifecycle3_founder_birth_warm_cuts_keep_complete_original_and_one_frame() {
    for (n, cut) in [
        cuts::Fault::Index3PartialStage,
        cuts::Fault::IndexStage,
        cuts::Fault::IndexRename,
        cuts::Fault::Index3Covered,
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xcb01 + n as u64);
        let mut owner = mounted(dir.path(), &mut rng);
        let old = selected(&owner);
        let mut input = pending(20);
        // The retained request's separate invoice covers BOTH maximum fields.
        input.name = "n".repeat(crate::store::critical_namespaces::native_registry::data::MAX_NAME);
        input.network_frame = Zeroizing::new(vec![55; 64 * 1024]);
        cuts::arm_fault(cut);
        assert!(owner.birth_founder_data(&input, &mut rng).is_err());
        assert!(cuts::take_fault_seen());
        for field in 0..9 {
            let mut other = input.clone();
            match field {
                0 => other.identity[0] ^= 1,
                1 => other.birth.invocation[0] ^= 1,
                2 => other.birth.request[0] ^= 1,
                3 => other.birth.signer[0] ^= 1,
                4 => other.birth.network[0] ^= 1,
                5 => other.name.replace_range(0..1, "x"),
                6 => other.is_dm = true,
                7 => other.network_frame[0] ^= 1,
                8 => {
                    other.birth.kind = BirthKind::Joined;
                    other.birth.join_context = Some([99; 32]);
                }
                _ => unreachable!(),
            }
            assert!(owner.birth_founder_data(&other, &mut NeverRng).is_err());
        }
        assert!(owner
            .rename_metadata_data(&request(&owner), &mut NeverRng)
            .is_err());
        let (result, debt) = owner.birth_founder_data(&input, &mut NeverRng).unwrap();
        let debt = debt.unwrap();
        assert_eq!(result.state, NativeCreationStateData::Pending);
        assert_eq!(result.id, old.last_issued_id + 1);
        assert!(owner.row_count_data().is_err());
        assert_eq!(
            fs::read(file(
                dir.path(),
                crate::store::critical_namespaces::native_registry::preparation::INDEX_NAME
            ))
            .unwrap(),
            *debt._transaction.frame
        );
        drop(debt);
        let new = selected(&owner);
        assert!(new.rows[..old.rows.len()] == old.rows);
        let (exact, no_debt) = owner.birth_founder_data(&input, &mut NeverRng).unwrap();
        assert_eq!(exact, result);
        assert!(no_debt.is_none());
    }
}

#[test]
fn lifecycle3_founder_activation_warm_cuts_keep_binding_and_neighbor_metadata() {
    for (n, cut) in [
        cuts::Fault::Index3PartialStage,
        cuts::Fault::IndexStage,
        cuts::Fault::IndexRename,
        cuts::Fault::Index3Covered,
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xcb11 + n as u64);
        let mut owner = mounted(dir.path(), &mut rng);
        let input = pending(30);
        born(&mut owner, &input, &mut rng);
        let act = activation(&owner, &input);
        let rename = request(&owner);
        drop(owner.rename_metadata_data(&rename, &mut rng).unwrap());
        let neighbor = selected(&owner);
        cuts::arm_fault(cut);
        assert!(owner.activate_founder_data(&act, &mut rng).is_err());
        assert!(cuts::take_fault_seen());
        for field in 0..10 {
            let mut other = act.clone();
            match field {
                0 => other.id += 1,
                1 => other.identity[0] ^= 1,
                2 => other.birth.request[0] ^= 1,
                3 => other.birth.network[0] ^= 1,
                4 => other.binding.vault[0] ^= 1,
                5 => other.binding.layout[0] ^= 1,
                6 => other.binding.group[0] ^= 1,
                7 => other.binding.source[0] ^= 1,
                8 => other.binding.profile[0] ^= 1,
                9 => other.binding.pin[0] ^= 1,
                _ => unreachable!(),
            }
            assert!(owner.activate_founder_data(&other, &mut NeverRng).is_err());
        }
        assert!(owner.birth_founder_data(&input, &mut NeverRng).is_err());
        let (result, debt) = owner.activate_founder_data(&act, &mut NeverRng).unwrap();
        assert_eq!(result.state, NativeCreationStateData::Founder);
        assert!(owner.row_count_data().is_err());
        drop(debt);
        let new = selected(&owner);
        assert!(new.rows[..neighbor.rows.len() - 1] == neighbor.rows[..neighbor.rows.len() - 1]);
        assert_eq!(new.last_issued_id, neighbor.last_issued_id);
        let (exact, no_debt) = owner.activate_founder_data(&act, &mut NeverRng).unwrap();
        assert_eq!(result, exact);
        assert!(no_debt.is_none());
    }
}

#[test]
fn lifecycle3_founder_cold_cuts_recover_birth_and_activation_without_rng() {
    for activating in [false, true] {
        for (n, cut) in [
            cuts::Fault::Index3PartialStage,
            cuts::Fault::IndexStage,
            cuts::Fault::IndexRename,
            cuts::Fault::Index3Covered,
        ]
        .into_iter()
        .enumerate()
        {
            let dir = tempfile::tempdir().unwrap();
            let mut rng = ChaCha20Rng::seed_from_u64(0xcb21 + n as u64);
            let mut owner = mounted(dir.path(), &mut rng);
            let input = pending(40);
            if activating {
                born(&mut owner, &input, &mut rng);
            }
            let old_count = selected(&owner).rows.len();
            let act = activating.then(|| activation(&owner, &input));
            cuts::arm_fault(cut);
            let result = if let Some(act) = &act {
                owner.activate_founder_data(act, &mut rng)
            } else {
                owner.birth_founder_data(&input, &mut rng)
            };
            assert!(result.is_err());
            assert!(cuts::take_fault_seen());
            drop(owner);
            let cold = reopen(dir.path());
            let recovered = selected(&cold);
            let row = recovered
                .lookup_data(input.birth.invocation, input.birth.request)
                .unwrap();
            if cut == cuts::Fault::Index3PartialStage {
                if activating {
                    assert!(matches!(&row.unwrap().backend, Backend::Pending { .. }));
                } else {
                    assert!(row.is_none());
                }
                assert_eq!(recovered.rows.len(), old_count);
            } else {
                let row = row.unwrap();
                if activating {
                    assert!(
                        matches!(&row.backend, Backend::Founder { binding, .. } if binding == &act.as_ref().unwrap().binding)
                    );
                } else {
                    assert!(
                        matches!(&row.backend, Backend::Pending { network_frame, .. } if network_frame == &input.network_frame)
                    );
                }
            }
        }
    }
}

#[test]
fn lifecycle3_founder_rejects_bad_bounds_conflicting_receipts_and_foreign_binding_before_rng() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xcb31);
    let mut owner = mounted(dir.path(), &mut rng);
    let input = pending(50);
    for field in 0..4 {
        let mut bad = input.clone();
        match field {
            0 => bad.name = "x".repeat(257),
            1 => bad.network_frame = Zeroizing::new(vec![1; 39]),
            2 => bad.network_frame = Zeroizing::new(vec![1; 64 * 1024 + 1]),
            3 => bad.birth.join_context = Some([1; 32]),
            _ => unreachable!(),
        }
        assert!(owner.birth_founder_data(&bad, &mut NeverRng).is_err());
    }
    born(&mut owner, &input, &mut rng);
    let mut changed = input.clone();
    changed.birth.request[0] ^= 1;
    assert!(owner.birth_founder_data(&changed, &mut NeverRng).is_err());
    let act = activation(&owner, &input);
    for field in 0..3 {
        let mut bad = act.clone();
        match field {
            0 => bad.binding.vault[0] ^= 1,
            1 => bad.binding.layout[0] ^= 1,
            2 => bad.binding.signer[0] ^= 1,
            _ => unreachable!(),
        }
        assert!(owner.activate_founder_data(&bad, &mut NeverRng).is_err());
    }
    drop(owner.activate_founder_data(&act, &mut rng).unwrap());
    let mut rename = request(&owner);
    rename.id = act.id;
    rename.identity = act.identity;
    rename.expected_revision = 0;
    drop(owner.rename_metadata_data(&rename, &mut rng).unwrap());
    let (_, no_debt) = owner.activate_founder_data(&act, &mut NeverRng).unwrap();
    assert!(
        no_debt.is_none(),
        "rename cannot invalidate immutable creation receipt"
    );
    assert_eq!(selected(&owner).rows.last().unwrap().name, rename.name);
}

#[test]
fn lifecycle3_founder_wal_preserves_valid_unsupported_joined_and_legacy_stages() {
    use super::super::super::{
        data::{JoinedBinding, LegacyOrigin},
        metadata::Plan,
        wal,
    };
    for kind in [BirthKind::Joined, BirthKind::LegacyJoin] {
        for activating in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut rng = ChaCha20Rng::seed_from_u64(0xcb41);
            let owner = mounted(dir.path(), &mut rng);
            let handler = owner.handler_for_test();
            let store = owner.actual_store_for_test().unwrap();
            let inventory =
                UpgradeInventory::capture(store, handler.debit(), handler.pool()).unwrap();
            let base = selected(&owner);
            let parent = *blake3::hash(inventory.index()).as_bytes();
            let input = pending(60);
            let Plan::Change { mut candidate, .. } = input.prepare(base, parent).unwrap() else {
                panic!("birth");
            };
            let row = candidate.rows.last_mut().unwrap();
            let Backend::Pending { birth, .. } = &mut row.backend else {
                panic!("pending");
            };
            birth.kind = kind;
            birth.join_context = Some([80; 32]);
            let birth = birth.clone();
            let seal_index = |index: &data::Index, rng: &mut ChaCha20Rng| {
                let key = Zeroizing::new(store.keys.db_key().unwrap());
                let derived =
                    Zeroizing::new(blake3::derive_key(preparation::INDEX_KEY, key.as_slice()));
                crate::store::frame(
                    &catcoms_crypto::seal(&derived, &index.encode_data().unwrap(), rng).unwrap(),
                )
            };
            let index_name =
                crate::store::critical_namespaces::native_registry::preparation::INDEX_NAME;
            let stage_name =
                crate::store::critical_namespaces::native_registry::preparation::INDEX_STAGE_NAME;
            if activating {
                // Deliberately install a structurally valid, MAC-valid future
                // Pending as test DATA. No joined producer/authority is claimed.
                let pending_frame = seal_index(&candidate, &mut rng);
                fs::write(file(dir.path(), index_name), &pending_frame).unwrap();
                candidate.generation += 1;
                candidate.parent = Some(*blake3::hash(&pending_frame).as_bytes());
                let vault = candidate.capability.original.vault;
                let layout = candidate.capability.original.root;
                candidate.rows.last_mut().unwrap().backend = match kind {
                    BirthKind::Joined => Backend::Joined {
                        birth: birth.clone(),
                        binding: JoinedBinding {
                            vault,
                            layout,
                            group: vec![81; 32],
                            signer: birth.signer,
                            source: [82; 32],
                            profile: [83; 32],
                            pin: [84; 32],
                            join_context: birth.join_context.unwrap(),
                            own_add: [85; 32],
                            initial_governing_tip: [86; 32],
                            initial_current_basis: [87; 32],
                            joined_epoch: 1,
                        },
                    },
                    BirthKind::LegacyJoin => Backend::Legacy {
                        origin: LegacyOrigin::Admitted(birth),
                        invite: "future invite".into(),
                    },
                    BirthKind::Founder => unreachable!(),
                };
            }
            // Full syntax, AEAD and general successor math are valid. Only the
            // CLOSED actual-producer classifier must refuse these transitions.
            let frame = seal_index(&candidate, &mut rng);
            drop(inventory);
            let inventory =
                UpgradeInventory::capture(store, handler.debit(), handler.pool()).unwrap();
            let old = preparation::decode_selected_data(store, &inventory)
                .unwrap()
                .0;
            let expected = if activating {
                data::Change::Activate
            } else {
                data::Change::Birth
            };
            assert_eq!(
                old.successor_data(&candidate, *blake3::hash(inventory.index()).as_bytes())
                    .unwrap(),
                expected
            );
            assert!(
                wal::verify_warm_metadata(store, &inventory, &candidate, &frame, expected).is_err()
            );
            drop(inventory);
            fs::write(file(dir.path(), stage_name), &frame).unwrap();
            let before = fs::read(file(dir.path(), index_name)).unwrap();
            drop(owner);
            assert!(NativeIndexStore::open_lifecycle3_for_test(
                dir.path(),
                SECRET,
                &mut NeverRng,
                current_policy()
            )
            .is_err());
            assert_eq!(fs::read(file(dir.path(), index_name)).unwrap(), before);
            assert_eq!(fs::read(file(dir.path(), stage_name)).unwrap(), frame);
        }
    }
}

#[test]
fn lifecycle3_founder_actual_cell_refuses_highwater_and_row_exhaustion_before_rng() {
    use super::super::super::data::{LegacyOrigin, Row};
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xcb51);
    let mut owner = mounted(dir.path(), &mut rng);
    let original = selected(&owner);
    let index_name = crate::store::critical_namespaces::native_registry::preparation::INDEX_NAME;
    let path = file(dir.path(), index_name);
    let original_frame = fs::read(&path).unwrap();
    for highwater in [true, false] {
        let mut exhausted =
            data::Index::decode_data(&original.encode_data().unwrap(), original.capability)
                .unwrap();
        exhausted.generation += 1;
        exhausted.parent = Some(*blake3::hash(&original_frame).as_bytes());
        if highwater {
            exhausted.last_issued_id = u64::MAX;
        } else {
            let count = crate::store::critical_namespaces::native_registry::data::MAX_ROWS;
            exhausted.rows = (1..=count as u64)
                .map(|id| {
                    let mut identity = [0; 32];
                    identity[..8].copy_from_slice(&id.to_be_bytes());
                    Row {
                        id,
                        identity,
                        is_dm: false,
                        name: "existing".into(),
                        metadata_revision: 0,
                        backend: Backend::Legacy {
                            origin: LegacyOrigin::Imported,
                            invite: "retained".into(),
                        },
                    }
                })
                .collect();
            exhausted.last_issued_id = count as u64;
        }
        exhausted.validate_data().unwrap();
        let key = Zeroizing::new(
            owner
                .actual_store_for_test()
                .unwrap()
                .keys
                .db_key()
                .unwrap(),
        );
        let derived = Zeroizing::new(blake3::derive_key(preparation::INDEX_KEY, key.as_slice()));
        let frame = crate::store::frame(
            &catcoms_crypto::seal(&derived, &exhausted.encode_data().unwrap(), &mut rng).unwrap(),
        );
        fs::write(&path, &frame).unwrap();
        assert!(owner
            .birth_founder_data(&pending(70), &mut NeverRng)
            .is_err());
        assert_eq!(
            fs::read(&path).unwrap(),
            frame,
            "no effect or new Stage on exhaustion"
        );
        assert!(!file(
            dir.path(),
            crate::store::critical_namespaces::native_registry::preparation::INDEX_STAGE_NAME
        )
        .exists());
        owner.handler_for_test().require_idle_cached().unwrap();
    }
    fs::write(&path, original_frame).unwrap();
    assert_eq!(owner.row_count_data().unwrap(), original.rows.len());
}

#[test]
fn lifecycle3_founder_index_close_on_both_edges_preserves_cold_receipts() {
    for activating in [false, true] {
        for edge in [Edge::BeforeMetadataAdmission, Edge::AfterMetadataCommit] {
            let dir = tempfile::tempdir().unwrap();
            let mut rng = ChaCha20Rng::seed_from_u64(0xcb61);
            let mut owner = mounted(dir.path(), &mut rng);
            let input = pending(80);
            if activating {
                born(&mut owner, &input, &mut rng);
            }
            let act = activating.then(|| activation(&owner, &input));
            let index = file(
                dir.path(),
                crate::store::critical_namespaces::native_registry::preparation::INDEX_NAME,
            );
            let before = fs::read(&index).unwrap();
            let closing = owner.handler_for_test();
            at_edge(edge, Box::new(move || closing.close()));
            let result = if let Some(act) = &act {
                owner.activate_founder_data(act, &mut rng)
            } else {
                owner.birth_founder_data(&input, &mut rng)
            };
            assert!(result.is_err(), "closed owner cannot report warm success");
            assert!(owner.birth_founder_data(&input, &mut NeverRng).is_err());
            if edge == Edge::BeforeMetadataAdmission {
                assert_eq!(fs::read(&index).unwrap(), before);
            }
            drop(owner);
            let cold = reopen(dir.path());
            let value = selected(&cold);
            let row = value
                .lookup_data(input.birth.invocation, input.birth.request)
                .unwrap();
            if activating {
                assert!(match &row.unwrap().backend {
                    Backend::Founder { .. } => edge == Edge::AfterMetadataCommit,
                    Backend::Pending { .. } => edge == Edge::BeforeMetadataAdmission,
                    _ => false,
                });
            } else {
                assert_eq!(row.is_some(), edge == Edge::AfterMetadataCommit);
            }
        }
    }
}

#[test]
fn lifecycle3_founder_post_effect_unwind_keeps_poison_until_cold_recovery() {
    for activating in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xcb71);
        let mut owner = mounted(dir.path(), &mut rng);
        let input = pending(90);
        if activating {
            born(&mut owner, &input, &mut rng);
        }
        let act = activating.then(|| activation(&owner, &input));
        at_edge(
            Edge::AfterMetadataCommit,
            Box::new(|| panic!("original founder effect unwinds")),
        );
        assert!(catch_unwind(AssertUnwindSafe(|| {
            if let Some(act) = &act {
                owner.activate_founder_data(act, &mut rng)
            } else {
                owner.birth_founder_data(&input, &mut rng)
            }
        }))
        .is_err());
        assert!(owner.row_count_data().is_err());
        assert!(owner.birth_founder_data(&input, &mut NeverRng).is_err());
        drop(owner);
        let cold = reopen(dir.path());
        let value = selected(&cold);
        let row = value
            .lookup_data(input.birth.invocation, input.birth.request)
            .unwrap()
            .unwrap();
        assert!(if activating {
            matches!(&row.backend, Backend::Founder { .. })
        } else {
            matches!(&row.backend, Backend::Pending { .. })
        });
    }
}
