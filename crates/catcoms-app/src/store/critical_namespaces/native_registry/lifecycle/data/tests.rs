//! Structural DATA tests, not authenticated migration, membership or runtime evidence.
use super::*;
use crate::store::critical_namespaces::archive_current_namespace_codec::CurrentLayoutData;

const FRAME: [u8; 32] = [90; 32];
fn base() -> Index {
    let current = CurrentLayoutData {
        vault: [1; 32],
        root: [2; 32],
    };
    let value = Index {
        capability: Capability::for_current_data(current, &current.encode_data().unwrap()).unwrap(),
        origin: MigrationOrigin {
            generation: 12,
            frame: [3; 32],
        },
        generation: 2,
        parent: Some(FRAME),
        last_issued_id: 2,
        rows: (1..=2)
            .map(|id| Row {
                id,
                identity: [id as u8; 32],
                is_dm: false,
                name: format!("group{id}"),
                metadata_revision: 0,
                backend: Backend::Legacy {
                    origin: LegacyOrigin::Imported,
                    invite: format!("invite{id}"),
                },
            })
            .collect(),
    };
    value.validate_data().unwrap();
    value
}
fn birth(kind: BirthKind) -> Birth {
    Birth {
        kind,
        invocation: [20; 32],
        request: [21; 32],
        signer: [22; 32],
        network: [23; 32],
        join_context: if kind == BirthKind::Founder {
            None
        } else {
            Some([24; 32])
        },
    }
}
fn next(old: &Index) -> Index {
    old.validate_data().unwrap();
    // Test-only bounded copies. Production must own the physical overlap fee;
    // no Clone/snapshot publication API is added to the actual row/index types.
    let rows = old
        .rows
        .iter()
        .map(|row| Row {
            id: row.id,
            identity: row.identity,
            is_dm: row.is_dm,
            name: row.name.clone(),
            metadata_revision: row.metadata_revision,
            backend: match &row.backend {
                Backend::Legacy { origin, invite } => Backend::Legacy {
                    origin: origin.clone(),
                    invite: invite.clone(),
                },
                Backend::RetiredLegacy {
                    origin,
                    invite,
                    terminal,
                } => Backend::RetiredLegacy {
                    origin: origin.clone(),
                    invite: invite.clone(),
                    terminal: *terminal,
                },
                Backend::Pending {
                    birth,
                    network_frame,
                } => Backend::Pending {
                    birth: birth.clone(),
                    network_frame: network_frame.clone(),
                },
                Backend::Founder { birth, binding } => Backend::Founder {
                    birth: birth.clone(),
                    binding: binding.clone(),
                },
                Backend::RetiredFounder {
                    birth,
                    binding,
                    terminal,
                } => Backend::RetiredFounder {
                    birth: birth.clone(),
                    binding: binding.clone(),
                    terminal: *terminal,
                },
                Backend::Joined { birth, binding } => Backend::Joined {
                    birth: birth.clone(),
                    binding: binding.clone(),
                },
                Backend::RetiredJoined {
                    birth,
                    binding,
                    terminal,
                } => Backend::RetiredJoined {
                    birth: birth.clone(),
                    binding: binding.clone(),
                    terminal: *terminal,
                },
                Backend::AbandonedFounder {
                    birth,
                    reason,
                    terminal,
                } => Backend::AbandonedFounder {
                    birth: birth.clone(),
                    reason: *reason,
                    terminal: *terminal,
                },
            },
        })
        .collect();
    Index {
        capability: old.capability,
        origin: old.origin,
        generation: old.generation + 1,
        parent: Some(FRAME),
        last_issued_id: old.last_issued_id,
        rows,
    }
}
fn joined(index: &Index) -> Backend {
    Backend::Joined {
        birth: birth(BirthKind::Joined),
        binding: JoinedBinding {
            vault: index.capability.original.vault,
            layout: index.capability.original.root,
            group: vec![4],
            signer: [22; 32],
            source: [25; 32],
            profile: [26; 32],
            pin: [27; 32],
            join_context: [24; 32],
            own_add: [28; 32],
            initial_governing_tip: [29; 32],
            initial_current_basis: [30; 32],
            joined_epoch: 7,
        },
    }
}

fn founder(index: &Index) -> Backend {
    Backend::Founder {
        birth: birth(BirthKind::Founder),
        binding: original::FounderBindingData {
            vault: index.capability.original.vault,
            layout: index.capability.original.root,
            group: vec![4],
            signer: [22; 32],
            source: [25; 32],
            profile: [26; 32],
            pin: [27; 32],
        },
    }
}

#[test]
fn lifecycle3_generation_one_accepts_only_exact_predecessor_shapes() {
    let mut initial = base();
    initial.generation = 1;
    initial.parent = None;
    initial.validate_data().unwrap();
    let retired_founder = match founder(&initial) {
        Backend::Founder { birth, binding } => Backend::RetiredFounder {
            birth,
            binding,
            terminal: [37; 32],
        },
        _ => unreachable!(),
    };
    let positive = [
        Backend::Pending {
            birth: birth(BirthKind::Founder),
            network_frame: Zeroizing::new(vec![5; 40]),
        },
        founder(&initial),
        retired_founder,
        Backend::AbandonedFounder {
            birth: birth(BirthKind::Founder),
            reason: original::AbandonedCreateReasonData::BeforeSdk,
            terminal: [32; 32],
        },
    ];
    for backend in positive {
        initial.rows[0].backend = backend;
        initial.validate_data().unwrap();
    }
    let joined = joined(&initial);
    let retired_joined = match &joined {
        Backend::Joined { birth, binding } => Backend::RetiredJoined {
            birth: birth.clone(),
            binding: binding.clone(),
            terminal: [38; 32],
        },
        _ => unreachable!(),
    };
    let invalid = [
        joined,
        retired_joined,
        Backend::Pending {
            birth: birth(BirthKind::Joined),
            network_frame: Zeroizing::new(vec![5; 40]),
        },
        Backend::Pending {
            birth: birth(BirthKind::LegacyJoin),
            network_frame: Zeroizing::new(vec![5; 40]),
        },
        Backend::Legacy {
            origin: LegacyOrigin::Admitted(birth(BirthKind::LegacyJoin)),
            invite: "not in v2".into(),
        },
        Backend::RetiredLegacy {
            origin: LegacyOrigin::Imported,
            invite: "invite1".into(),
            terminal: [33; 32],
        },
    ];
    for backend in invalid {
        initial.rows[0].backend = backend;
        assert!(initial.validate_data().is_err());
    }
    initial.rows[0].backend = founder(&initial);
    initial.rows[0].metadata_revision = 1;
    assert!(initial.validate_data().is_err());
    initial.rows[0].metadata_revision = 0;
    initial.origin.generation = 1;
    assert!(
        initial.validate_data().is_err(),
        "old index2 G1 was Legacy-only"
    );
    initial.rows[0].backend = Backend::Legacy {
        origin: LegacyOrigin::Imported,
        invite: "invite1".into(),
    };
    initial.validate_data().unwrap();
    initial.last_issued_id += 1;
    assert!(
        initial.validate_data().is_err(),
        "old index2 G1 highwater was exact"
    );
    initial.rows.clear();
    assert!(initial.validate_data().is_err());
    initial.last_issued_id = 0;
    initial.validate_data().unwrap();
}

#[test]
fn lifecycle3_native_retirement_preserves_binding_and_prevents_resurrection() {
    for is_joined in [false, true] {
        let mut old = base();
        old.rows[0].backend = if is_joined {
            joined(&old)
        } else {
            founder(&old)
        };
        let mut retired = next(&old);
        retired.rows[0].backend = match &old.rows[0].backend {
            Backend::Founder { birth, binding } => Backend::RetiredFounder {
                birth: birth.clone(),
                binding: binding.clone(),
                terminal: [34; 32],
            },
            Backend::Joined { birth, binding } => Backend::RetiredJoined {
                birth: birth.clone(),
                binding: binding.clone(),
                terminal: [34; 32],
            },
            _ => unreachable!(),
        };
        assert_eq!(old.successor_data(&retired, FRAME).unwrap(), Change::Retire);
        let mut resurrection = next(&retired);
        resurrection.rows[0].backend = next(&old).rows.remove(0).backend;
        assert!(retired.successor_data(&resurrection, FRAME).is_err());
        if let Backend::RetiredJoined { binding, .. } = &mut retired.rows[0].backend {
            binding.initial_governing_tip[0] ^= 1;
        } else if let Backend::RetiredFounder { binding, .. } = &mut retired.rows[0].backend {
            binding.profile[0] ^= 1;
        }
        assert!(
            old.successor_data(&retired, FRAME).is_err(),
            "retirement must preserve the full initial binding"
        );
    }
}

#[test]
fn lifecycle3_join_attempts_cannot_reinterpret_founder_abandonment_reasons() {
    for kind in [BirthKind::Joined, BirthKind::LegacyJoin] {
        let mut old = base();
        old.rows[0].backend = Backend::Pending {
            birth: birth(kind),
            network_frame: Zeroizing::new(vec![5; 40]),
        };
        let mut abandoned = next(&old);
        abandoned.rows[0].backend = Backend::AbandonedFounder {
            birth: birth(kind),
            reason: original::AbandonedCreateReasonData::UnexposedAfterSdk,
            terminal: [35; 32],
        };
        assert!(abandoned.validate_data().is_err());
        assert!(old.successor_data(&abandoned, FRAME).is_err());
    }
}

#[test]
fn lifecycle3_rename_changes_one_active_name_and_exact_revision_only() {
    let old = base();
    let mut candidate = next(&old);
    candidate.rows[0].name = "renamed".into();
    candidate.rows[0].metadata_revision = 1;
    assert_eq!(
        old.successor_data(&candidate, FRAME).unwrap(),
        Change::Rename
    );
    candidate.rows[1].name = "another rename".into();
    candidate.rows[1].metadata_revision = 1;
    assert!(old.successor_data(&candidate, FRAME).is_err());
    candidate = next(&old);
    candidate.rows[0].name = "stale".into();
    assert!(old.successor_data(&candidate, FRAME).is_err());
    candidate.rows[0].metadata_revision = 1;
    candidate.rows[0].is_dm = true;
    assert!(old.successor_data(&candidate, FRAME).is_err());
}

#[test]
fn lifecycle3_invite_refresh_is_distinct_from_rename_and_checks_shared_revision() {
    let old = base();
    let mut candidate = next(&old);
    let Backend::Legacy { invite, .. } = &mut candidate.rows[0].backend else {
        unreachable!()
    };
    *invite = "fresh native-minted invite".into();
    candidate.rows[0].metadata_revision = 1;
    assert_eq!(
        old.successor_data(&candidate, FRAME).unwrap(),
        Change::RefreshLegacyInvite
    );
    candidate.rows[0].name = "combined mutation".into();
    assert!(old.successor_data(&candidate, FRAME).is_err());
    candidate.rows[0].name = old.rows[0].name.clone();
    candidate.rows[0].metadata_revision = 0;
    assert!(old.successor_data(&candidate, FRAME).is_err());
}

#[test]
fn lifecycle3_legacy_retirement_preserves_origin_invite_and_never_resurrects() {
    let old = base();
    let mut retired = next(&old);
    retired.rows[0].backend = Backend::RetiredLegacy {
        origin: LegacyOrigin::Imported,
        invite: "invite1".into(),
        terminal: [31; 32],
    };
    assert_eq!(old.successor_data(&retired, FRAME).unwrap(), Change::Retire);
    let mut resurrection = next(&retired);
    resurrection.rows[0].backend = Backend::Legacy {
        origin: LegacyOrigin::Imported,
        invite: "invite1".into(),
    };
    assert!(retired.successor_data(&resurrection, FRAME).is_err());
    let mut deletion = next(&retired);
    deletion.rows.remove(0);
    assert!(retired.successor_data(&deletion, FRAME).is_err());
    let mut rename = next(&retired);
    rename.rows[0].name = "hidden history".into();
    rename.rows[0].metadata_revision += 1;
    assert!(retired.successor_data(&rename, FRAME).is_err());
}

#[test]
fn lifecycle3_birth_requires_pending_monotone_id_and_unchanged_prefix() {
    let old = base();
    let mut candidate = next(&old);
    candidate.rows.push(Row {
        id: 3,
        identity: [40; 32],
        is_dm: false,
        name: "new".into(),
        metadata_revision: 0,
        backend: Backend::Pending {
            birth: birth(BirthKind::Founder),
            network_frame: Zeroizing::new(vec![5; 40]),
        },
    });
    candidate.last_issued_id = 3;
    assert_eq!(
        old.successor_data(&candidate, FRAME).unwrap(),
        Change::Birth
    );
    candidate.rows[0].name = "changed prefix".into();
    assert!(old.successor_data(&candidate, FRAME).is_err());
    candidate.rows[0].name = old.rows[0].name.clone();
    candidate.rows[2].id = 4;
    candidate.last_issued_id = 4;
    assert!(old.successor_data(&candidate, FRAME).is_err());
}

#[test]
fn lifecycle3_legacy_join_activation_retains_its_real_receipt_kind() {
    let mut old = base();
    old.rows[0].backend = Backend::Pending {
        birth: birth(BirthKind::LegacyJoin),
        network_frame: Zeroizing::new(vec![5; 40]),
    };
    let mut active = next(&old);
    active.rows[0].backend = Backend::Legacy {
        origin: LegacyOrigin::Admitted(birth(BirthKind::LegacyJoin)),
        invite: "joined".into(),
    };
    assert_eq!(
        old.successor_data(&active, FRAME).unwrap(),
        Change::Activate
    );
    active.rows[0].backend = Backend::Legacy {
        origin: LegacyOrigin::Imported,
        invite: "lost receipt".into(),
    };
    assert!(old.successor_data(&active, FRAME).is_err());
}

#[test]
fn lifecycle3_joined_binding_checks_full_signer_context_and_original_layout() {
    let mut old = base();
    old.rows[0].backend = Backend::Pending {
        birth: birth(BirthKind::Joined),
        network_frame: Zeroizing::new(vec![5; 40]),
    };
    let mut active = next(&old);
    active.rows[0].backend = joined(&old);
    assert_eq!(
        old.successor_data(&active, FRAME).unwrap(),
        Change::Activate
    );
    active.validate_data().unwrap();
    let Backend::Joined { binding, .. } = &mut active.rows[0].backend else {
        unreachable!()
    };
    binding.signer[0] ^= 1;
    assert!(old.successor_data(&active, FRAME).is_err());
    active.rows[0].backend = joined(&old);
    let Backend::Joined { binding, .. } = &mut active.rows[0].backend else {
        unreachable!()
    };
    binding.join_context[0] ^= 1;
    assert!(old.successor_data(&active, FRAME).is_err());
    active.rows[0].backend = joined(&old);
    let Backend::Joined { binding, .. } = &mut active.rows[0].backend else {
        unreachable!()
    };
    binding.layout[0] ^= 1;
    assert!(old.successor_data(&active, FRAME).is_err());
}

#[test]
fn lifecycle3_lookup_keeps_creation_identity_after_rename_and_reports_conflict() {
    let mut old = base();
    old.rows[0].backend = joined(&old);
    let mut renamed = next(&old);
    renamed.rows[0].name = "new display only".into();
    renamed.rows[0].metadata_revision = 1;
    assert_eq!(old.successor_data(&renamed, FRAME).unwrap(), Change::Rename);
    assert_eq!(
        renamed
            .lookup_data([20; 32], [21; 32])
            .unwrap()
            .unwrap()
            .name,
        "new display only"
    );
    assert!(renamed.lookup_data([20; 32], [99; 32]).is_err());
    assert!(renamed.lookup_data([99; 32], [21; 32]).unwrap().is_none());
}

#[test]
fn lifecycle3_successors_cannot_rebase_origin_parent_highwater_or_backend() {
    let old = base();
    let mut candidate = next(&old);
    candidate.rows[0].name = "rename".into();
    candidate.rows[0].metadata_revision = 1;
    candidate.origin.frame[0] ^= 1;
    assert!(old.successor_data(&candidate, FRAME).is_err());
    candidate.origin = old.origin;
    candidate.parent = Some([99; 32]);
    assert!(old.successor_data(&candidate, FRAME).is_err());
    candidate.parent = Some(FRAME);
    candidate.last_issued_id -= 1;
    assert!(old.successor_data(&candidate, FRAME).is_err());
    candidate = next(&old);
    candidate.rows[0].backend = joined(&old);
    assert!(old.successor_data(&candidate, FRAME).is_err());
}

#[test]
fn lifecycle3_structure_rejects_duplicate_receipts_and_unsupported_birth_context() {
    let mut value = base();
    value.rows[0].backend = joined(&value);
    value.rows[1].backend = joined(&value);
    assert!(value.validate_data().is_err());
    value = base();
    value.rows[1].identity = value.rows[0].identity;
    assert!(value.validate_data().is_err());
    value = base();
    let mut invalid = birth(BirthKind::Founder);
    invalid.join_context = Some([0; 32]);
    value.rows[0].backend = Backend::Pending {
        birth: invalid,
        network_frame: Zeroizing::new(vec![5; 40]),
    };
    assert!(value.validate_data().is_err());
}

#[test]
fn lifecycle3_founder_activation_and_abandonment_preserve_the_original_token() {
    let mut old = base();
    old.rows[0].backend = Backend::Pending {
        birth: birth(BirthKind::Founder),
        network_frame: Zeroizing::new(vec![5; 40]),
    };
    let mut active = next(&old);
    active.rows[0].backend = founder(&old);
    assert_eq!(
        old.successor_data(&active, FRAME).unwrap(),
        Change::Activate
    );
    assert_eq!(
        active.lookup_data([20; 32], [21; 32]).unwrap().unwrap().id,
        1
    );
    let mut abandoned = next(&old);
    abandoned.rows[0].backend = Backend::AbandonedFounder {
        birth: birth(BirthKind::Founder),
        reason: original::AbandonedCreateReasonData::BeforeSdk,
        terminal: [36; 32],
    };
    assert_eq!(
        old.successor_data(&abandoned, FRAME).unwrap(),
        Change::Abandon
    );
    assert_eq!(
        abandoned
            .lookup_data([20; 32], [21; 32])
            .unwrap()
            .unwrap()
            .id,
        1
    );
    let mut resurrection = next(&abandoned);
    resurrection.rows[0].backend = founder(&old);
    assert!(abandoned.successor_data(&resurrection, FRAME).is_err());
    let mut changed_receipt = next(&old);
    changed_receipt.rows[0].backend = founder(&old);
    let Backend::Founder { birth, .. } = &mut changed_receipt.rows[0].backend else {
        unreachable!()
    };
    birth.network[0] ^= 1;
    assert!(old.successor_data(&changed_receipt, FRAME).is_err());
}

#[test]
fn lifecycle3_generation_metadata_and_id_exhaustion_never_wrap_or_rebase() {
    let mut old = base();
    old.generation = u64::MAX;
    old.validate_data().unwrap();
    let mut candidate = base();
    candidate.generation = u64::MAX;
    candidate.rows[0].name = "rename at exhausted generation".into();
    candidate.rows[0].metadata_revision = 1;
    candidate.validate_data().unwrap();
    assert!(old.successor_data(&candidate, FRAME).is_err());
    old = base();
    old.rows[0].metadata_revision = u64::MAX;
    candidate = next(&old);
    candidate.rows[0].name = "cannot reset revision".into();
    candidate.rows[0].metadata_revision = 0;
    candidate.validate_data().unwrap();
    assert!(old.successor_data(&candidate, FRAME).is_err());
    old = base();
    old.last_issued_id = u64::MAX;
    candidate = next(&old);
    candidate.rows.push(Row {
        id: u64::MAX,
        identity: [41; 32],
        is_dm: false,
        name: "not a new ceiling".into(),
        metadata_revision: 0,
        backend: Backend::Pending {
            birth: birth(BirthKind::Founder),
            network_frame: Zeroizing::new(vec![5; 40]),
        },
    });
    candidate.validate_data().unwrap();
    assert!(old.successor_data(&candidate, FRAME).is_err());
}

#[test]
fn lifecycle3_codec_roundtrips_every_closed_backend_and_birth_kind() {
    let mut value = base();
    let retired_founder = match founder(&value) {
        Backend::Founder { birth, binding } => Backend::RetiredFounder {
            birth,
            binding,
            terminal: [42; 32],
        },
        _ => unreachable!(),
    };
    let retired_joined = match joined(&value) {
        Backend::Joined { birth, binding } => Backend::RetiredJoined {
            birth,
            binding,
            terminal: [43; 32],
        },
        _ => unreachable!(),
    };
    let cases = [
        Backend::Legacy {
            origin: LegacyOrigin::Imported,
            invite: "kept invite".into(),
        },
        Backend::Legacy {
            origin: LegacyOrigin::Admitted(birth(BirthKind::LegacyJoin)),
            invite: "new invite".into(),
        },
        Backend::RetiredLegacy {
            origin: LegacyOrigin::Imported,
            invite: "old invite".into(),
            terminal: [44; 32],
        },
        Backend::RetiredLegacy {
            origin: LegacyOrigin::Admitted(birth(BirthKind::LegacyJoin)),
            invite: "retained invite".into(),
            terminal: [45; 32],
        },
        Backend::Pending {
            birth: birth(BirthKind::Founder),
            network_frame: Zeroizing::new(vec![5; 40]),
        },
        Backend::Pending {
            birth: birth(BirthKind::Joined),
            network_frame: Zeroizing::new(vec![6; 64 * 1024]),
        },
        Backend::Pending {
            birth: birth(BirthKind::LegacyJoin),
            network_frame: Zeroizing::new(vec![7; 40]),
        },
        founder(&value),
        retired_founder,
        joined(&value),
        retired_joined,
        Backend::AbandonedFounder {
            birth: birth(BirthKind::Founder),
            reason: original::AbandonedCreateReasonData::BeforeSdk,
            terminal: [46; 32],
        },
        Backend::AbandonedFounder {
            birth: birth(BirthKind::Founder),
            reason: original::AbandonedCreateReasonData::UnexposedAfterSdk,
            terminal: [47; 32],
        },
    ];
    for backend in cases {
        value.rows[0].backend = backend;
        value.rows[0].name = "snow 雪".into();
        value.rows[0].metadata_revision = 1;
        let encoded = value.encode_data().unwrap();
        assert_eq!(encoded.len(), value.encoded_length().unwrap());
        let decoded = Index::decode_data(&encoded, value.capability).unwrap();
        assert!(decoded == value);
        assert_eq!(
            decoded.encode_data().unwrap().as_slice(),
            encoded.as_slice()
        );
    }
}

#[test]
fn lifecycle3_codec_rejects_truncation_unknown_tags_flags_counts_and_foreign_context() {
    let value = base();
    let encoded = value.encode_data().unwrap();
    for cut in 0..encoded.len() {
        assert!(Index::decode_data(&encoded[..cut], value.capability).is_err());
    }
    let mut trailing = encoded.to_vec();
    trailing.push(0);
    assert!(Index::decode_data(&trailing, value.capability).is_err());
    let version = 4 + b"catcoms/private-native-backend-index/v3\0".len();
    let origin = version + 2 + 3 * 36;
    let parent_flag = origin + 45 + 8;
    let count = parent_flag + 1 + 36 + 8;
    let row = count + 4;
    for offset in [version, origin, parent_flag, row, row + 1 + 8 + 36] {
        let mut malformed = encoded.to_vec();
        malformed[offset] = 99;
        assert!(Index::decode_data(&malformed, value.capability).is_err());
    }
    let mut too_many = encoded.to_vec();
    too_many[count..count + 4].fill(255);
    assert!(Index::decode_data(&too_many, value.capability).is_err());
    let mut foreign = value.capability;
    foreign.original.vault[0] ^= 1;
    assert!(Index::decode_data(&encoded, foreign).is_err());
    let mut full_id = encoded.to_vec();
    full_id[version + 2 + 2 * 36 + 4] ^= 1;
    assert!(Index::decode_data(&full_id, value.capability).is_err());
    let mut predecessor = encoded.to_vec();
    predecessor[version - 2] = b'2';
    assert!(Index::decode_data(&predecessor, value.capability).is_err());
}

#[test]
fn lifecycle3_codec_enforces_aggregate_and_per_field_limits_before_encoding() {
    let mut value = base();
    value.rows[0].name = "x".repeat(original::MAX_NAME + 1);
    assert!(value.encode_data().is_err());
    value = base();
    value.rows[0].backend = Backend::Pending {
        birth: birth(BirthKind::Founder),
        network_frame: Zeroizing::new(vec![1; 64 * 1024 + 1]),
    };
    assert!(value.encode_data().is_err());
    value = base();
    value.rows = (1u64..=72)
        .map(|id| {
            let mut identity = [0; 32];
            identity[..8].copy_from_slice(&id.to_be_bytes());
            Row {
                id,
                identity,
                is_dm: false,
                name: "x".repeat(original::MAX_NAME),
                metadata_revision: 0,
                backend: Backend::Legacy {
                    origin: LegacyOrigin::Imported,
                    invite: "i".repeat(original::MAX_INVITE),
                },
            }
        })
        .collect();
    value.last_issued_id = 72;
    value.validate_data().unwrap();
    assert!(
        value.encoded_length().is_err(),
        "bounded fields can still exceed the complete frame budget"
    );
    // Exact neighboring boundary, with an already-present parent: each full
    // Legacy row is65855 bytes and owes another36 bytes for its terminal.
    let row_bytes = 1 + 8 + 36 + 1 + 4 + original::MAX_NAME + 8 + 1 + 4 + original::MAX_INVITE;
    let mut empty = base();
    empty.rows.clear();
    let header = empty.encoded_length().unwrap();
    assert!(header + 72 * (row_bytes + 36) > super::super::codec::MAX_PLAIN);
    value.rows.pop();
    value.last_issued_id = 71;
    assert_eq!(value.encode_data().unwrap().len(), header + 71 * row_bytes);
    assert_eq!(
        value.reserved_length_data().unwrap(),
        header + 71 * (row_bytes + 36)
    );
    assert!(value.reserved_length_data().unwrap() <= super::super::codec::MAX_PLAIN);
    let oversized = vec![0; super::super::codec::MAX_PLAIN + 1];
    assert!(Index::decode_data(&oversized, value.capability).is_err());
}

#[test]
fn lifecycle3_cold_generation_one_decoder_rejects_a_handwritten_joined_shortcut() {
    let mut value = base();
    value.rows[0].backend = joined(&value);
    let mut raw = value.encode_data().unwrap().to_vec();
    let generation = 4 + b"catcoms/private-native-backend-index/v3\0".len() + 2 + 3 * 36 + 45;
    raw[generation..generation + 8].copy_from_slice(&1u64.to_be_bytes());
    let parent = generation + 8;
    raw[parent] = 0;
    raw.drain(parent + 1..parent + 1 + 36);
    assert!(Index::decode_data(&raw, value.capability).is_err());
}

#[test]
fn lifecycle3_births_and_metadata_cannot_spend_the_selected_terminal_reservation() {
    let mut value = base();
    value.rows = (1u64..=72)
        .map(|id| {
            let mut identity = [0; 32];
            identity[..8].copy_from_slice(&id.to_be_bytes());
            Row {
                id,
                identity,
                is_dm: false,
                name: String::new(),
                metadata_revision: 0,
                backend: Backend::Legacy {
                    origin: LegacyOrigin::Imported,
                    invite: String::new(),
                },
            }
        })
        .collect();
    value.last_issued_id = 72;
    let target = super::super::codec::MAX_PLAIN - 100;
    let mut remaining = target - value.reserved_length_data().unwrap();
    for row in &mut value.rows {
        let Backend::Legacy { invite, .. } = &mut row.backend else {
            unreachable!()
        };
        let bytes = remaining.min(original::MAX_INVITE);
        *invite = "i".repeat(bytes);
        remaining -= bytes;
    }
    assert_eq!(remaining, 0);
    assert_eq!(value.reserved_length_data().unwrap(), target);
    let actual = value.encode_data().unwrap().len();
    // Each new40B encrypted Pending fits today's actual bytes. It MUST still
    // be refused if its own full activation/terminal would spend earlier debt.
    for kind in [BirthKind::Founder, BirthKind::Joined, BirthKind::LegacyJoin] {
        let mut pending = next(&value);
        pending.rows.push(Row {
            id: 73,
            identity: [48; 32],
            name: "new".into(),
            is_dm: false,
            metadata_revision: 0,
            backend: Backend::Pending {
                birth: birth(kind),
                network_frame: Zeroizing::new(vec![1; 40]),
            },
        });
        pending.last_issued_id = 73;
        pending.validate_data().unwrap();
        assert!(actual + 300 < super::super::codec::MAX_PLAIN);
        assert!(pending.encode_data().is_err());
        assert!(value.successor_data(&pending, FRAME).is_err());
    }
    let mut rename = next(&value);
    rename.rows[0].name = "x".repeat(201);
    rename.rows[0].metadata_revision = 1;
    rename.validate_data().unwrap();
    assert!(actual + 201 < super::super::codec::MAX_PLAIN);
    assert!(rename.encode_data().is_err());
    assert!(value.successor_data(&rename, FRAME).is_err());
    let mut retired = next(&value);
    let Backend::Legacy { origin, invite } = &value.rows[0].backend else {
        unreachable!()
    };
    retired.rows[0].backend = Backend::RetiredLegacy {
        origin: origin.clone(),
        invite: invite.clone(),
        terminal: [49; 32],
    };
    assert_eq!(
        value.successor_data(&retired, FRAME).unwrap(),
        Change::Retire
    );
    assert_eq!(retired.reserved_length_data().unwrap(), target);
    retired.encode_data().unwrap();
}
