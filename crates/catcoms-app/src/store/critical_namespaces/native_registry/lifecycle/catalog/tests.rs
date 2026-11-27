//! Pure syntax fixtures, not authenticated sources or execution authorities.
//! Real held-file/JOIN lifecycle tests belong to the physical upgrade.
use super::*;
use crate::store::critical_namespaces::{
    archive_bootstrap,
    archive_current_namespace_codec::CurrentLayoutData,
    catalog_declaration,
    pending_join::{execution_payload, execution_state},
};

const VAULT: [u8; 32] = [0x73; 32];

fn original(join_version: Option<u32>, paused: bool) -> (Vec<u8>, NativeCapabilityData) {
    let layout = CurrentLayoutData {
        vault: VAULT,
        root: [0x74; 32],
    };
    let current = layout.encode_data().unwrap();
    let cap = NativeCapabilityData::for_current_data(layout, &current).unwrap();
    let mut out = Encoder::new();
    out.put_str(outer::TAG_V2).unwrap();
    out.put_bytes(&VAULT).unwrap();
    out.put_u64(9);
    out.put_bytes(&[0x75; 32]).unwrap();
    out.put_u32(2 + u32::from(join_version.is_some()));
    if let Some(version) = join_version {
        out.put_u32(1).put_u32(version);
        out.put_bytes(&join(version, paused)).unwrap();
    }
    let archive = archive_bootstrap::Descriptor {
        vault: VAULT,
        root: layout.root,
    }
    .encode()
    .unwrap();
    for (id, bytes) in [(2, archive.as_slice()), (3, current.as_slice())] {
        out.put_u32(id).put_u32(1);
        out.put_bytes(bytes).unwrap();
    }
    let member =
        crate::store::critical_namespaces::member_outbox_namespace_codec::prepare_declaration_data(
            &out.finish(),
            &VAULT,
        )
        .unwrap();
    let native = catalog_declaration::insert_entry_data(
        &member,
        &VAULT,
        7,
        2,
        &v2::Capability { original: cap }.encode_data().unwrap(),
    )
    .unwrap();
    (native.to_vec(), cap)
}

fn join(version: u32, paused: bool) -> Vec<u8> {
    let mut immutable = Encoder::new();
    immutable.put_str(grammar::IMMUTABLE_TAG).unwrap();
    immutable.put_bytes(&VAULT).unwrap();
    immutable.put_bytes(&[0x76; 32]).unwrap();
    immutable.put_u8(1).put_u8(1);
    for bytes in [
        &b"retained join"[..],
        &[0x77; 32],
        &[0x78; 32],
        &[0x79; 32],
        &[0x7a; 16],
        &[0x7b; 32],
        &[0x7c; 32],
        &[0x7d; 32],
    ] {
        immutable.put_bytes(bytes).unwrap();
    }
    immutable.put_bytes(&[0]).unwrap();
    let immutable = immutable.finish();
    let identity = execution_state::SourceIdentity {
        binding: grammar::binding_digest(&immutable),
        request: [0x7c; 32],
        package: [0x7d; 32],
    };
    let mut row = Encoder::new();
    row.put_bytes(&[0x76; 32]).unwrap();
    row.put_u64(3).put_u8(u8::from(paused)).put_u8(1);
    row.put_bytes(&immutable).unwrap();
    row.put_bytes(&[0x7e; 40]).unwrap();
    let row = if version == 2 {
        // A spent attempt is history too. The migration must not reset it to
        // NotGranted or infer that a paused source was never submitted.
        let state = execution_state::State {
            grant: 1,
            limit: 2,
            spent: 1,
            ordinal: 1,
            arm: Some(execution_state::Arm {
                source: identity,
                grant: 1,
                ordinal: 1,
                scheduling_revision: 2,
                activation: [0x7f; 32],
                invocation: [0x80; 32],
                deadline_ms: 1000,
            }),
            input: None,
        };
        let mut execution = Encoder::new();
        execution_state::encode_into(&state, None, &identity, 3, &mut execution).unwrap();
        let mut nested = Encoder::new();
        nested.put_bytes(&row.finish()).unwrap();
        nested.put_bytes(&execution.finish()).unwrap();
        nested.finish()
    } else {
        row.finish()
    };
    let mut payload = Encoder::new();
    payload
        .put_str(if version == 2 {
            execution_payload::TAG
        } else {
            grammar::JOIN_TAG
        })
        .unwrap();
    payload.put_u32(1);
    payload.put_bytes(&row).unwrap();
    payload.finish()
}

fn assert_pair(before: &[u8], after: &[u8], cap: NativeCapabilityData) {
    let a = outer::parse(before, &VAULT, outer::MAX_ENVELOPE_PLAIN).unwrap();
    let b = outer::parse(after, &VAULT, outer::MAX_ENVELOPE_PLAIN).unwrap();
    assert_eq!(b.generation, a.generation + 1);
    assert_eq!(b.parent, Some(a.digest));
    assert_eq!(b.count, a.count);
    for index in 0..a.count {
        let old = a.entry(index).unwrap();
        let new = b.entry(index).unwrap();
        assert_eq!(new.id, old.id);
        if old.id == 7 {
            assert_eq!((old.version, new.version), (2, 3));
            Capability::decode_exact(new.payload, Capability { original: cap }).unwrap();
        } else {
            assert_eq!(new.version, old.version);
            assert_eq!(new.payload, old.payload);
        }
    }
    assert!(grammar::parse_native_registry_layout(after, &VAULT, true, false).is_err());
    assert!(grammar::parse_operational_native_registry_layout(after, &VAULT, true).is_err());
    assert!(grammar::parse_lifecycle_native_registry_layout(before, &VAULT, true).is_err());
    grammar::parse_lifecycle_native_registry_layout(after, &VAULT, true).unwrap();
}

#[test]
fn lifecycle3_catalog_upgrade_preserves_all_active_paused_and_spent_join_bytes() {
    for version in [None, Some(1), Some(2)] {
        for paused in [false, true] {
            let (before, cap) = original(version, paused);
            let after = upgrade_data(&before, &VAULT, cap).unwrap();
            assert_pair(&before, &after, cap);
            let parsed =
                grammar::parse_lifecycle_native_registry_layout(&after, &VAULT, true).unwrap();
            assert_eq!(parsed.count, usize::from(version.is_some()));
            assert_eq!(parsed.join_version, version);
            if let Some(version) = version {
                let row = &parsed.rows[0].as_ref().unwrap().metadata;
                assert_eq!(row.scheduling == grammar::Scheduling::Paused, paused);
                assert_eq!(row.revision, 3);
                if version == 2 {
                    assert_eq!(row.execution.unwrap().spent, 1);
                    assert!(
                        grammar::parse_lifecycle_native_registry_layout(&after, &VAULT, false)
                            .is_err()
                    );
                }
            }
        }
    }
}

#[test]
fn lifecycle3_catalog_upgrade_refuses_foreign_context_repeat_and_generation_exhaustion() {
    let (before, cap) = original(Some(2), true);
    let after = upgrade_data(&before, &VAULT, cap).unwrap();
    assert!(upgrade_data(&after, &VAULT, cap).is_err());
    assert!(upgrade_data(&before, &[0x99; 32], cap).is_err());
    let layout = CurrentLayoutData {
        vault: VAULT,
        root: [0x99; 32],
    };
    let foreign =
        NativeCapabilityData::for_current_data(layout, &layout.encode_data().unwrap()).unwrap();
    assert!(upgrade_data(&before, &VAULT, foreign).is_err());
    let mut exhausted = before;
    let at = 4 + outer::TAG_V2.len() + 36;
    exhausted[at..at + 8].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(upgrade_data(&exhausted, &VAULT, cap).is_err());
}

#[test]
fn lifecycle3_catalog_upgrade_refuses_unknown_entries_and_every_truncated_original() {
    let (before, cap) = original(None, false);
    let future = catalog_declaration::insert_entry_data(&before, &VAULT, 9, 1, b"unknown").unwrap();
    assert!(upgrade_data(&future, &VAULT, cap).is_err());
    for length in 0..before.len() {
        assert!(
            upgrade_data(&before[..length], &VAULT, cap).is_err(),
            "cut {length}"
        );
    }
    let mut trailing = before;
    trailing.push(0);
    assert!(upgrade_data(&trailing, &VAULT, cap).is_err());
}

fn joined_catalog() -> (Zeroizing<Vec<u8>>, NativeCapabilityData) {
    use crate::store::critical_namespaces::{
        governance_namespace_codec::{prepare_family_declaration_data, GovernanceCatalogFamily},
        joined_pin::namespace as joined,
        shared_history_namespace as history,
    };
    let (before, cap) = original(None, false);
    let native = upgrade_data(&before, &VAULT, cap).unwrap();
    let governed =
        prepare_family_declaration_data(&native, &VAULT, GovernanceCatalogFamily::NativeLifecycle)
            .unwrap();
    let joined = joined::prepare_declaration(&governed, &VAULT).unwrap();
    assert!(history::prepare_declaration(&governed, &VAULT).is_err());
    (joined, cap)
}

#[test]
fn shared_history_declaration_only_adds_eight_and_predecessor_refuses() {
    use crate::store::critical_namespaces::shared_history_namespace as history;
    let (joined, _) = joined_catalog();
    let prior =
        catalog_declaration::insert_entry_data(&joined, &VAULT, 1, 2, &join(2, true)).unwrap();
    let declaration = history::prepare_declaration(&prior, &VAULT).unwrap();
    history::verify_declaration(&prior, &declaration, &VAULT).unwrap();
    let original = outer::parse(&prior, &VAULT, outer::MAX_ENVELOPE_PLAIN).unwrap();
    let next = outer::parse(&declaration, &VAULT, outer::MAX_ENVELOPE_PLAIN).unwrap();
    assert_eq!(next.count, original.count + 1);
    assert_eq!(next.parent, Some(original.digest));
    for index in 0..original.count {
        let old = original.entry(index).unwrap();
        let new = next.entry(index).unwrap();
        assert_eq!(
            (old.id, old.version, old.payload),
            (new.id, new.version, new.payload)
        );
    }
    let parsed =
        grammar::parse_shared_history_native_registry_layout(&declaration, &VAULT, true).unwrap();
    let row = &parsed.rows[0].as_ref().unwrap().metadata;
    assert!(row.scheduling == grammar::Scheduling::Paused);
    assert_eq!(row.execution.unwrap().spent, 1);
    assert!(grammar::parse_joined_native_registry_layout(&declaration, &VAULT, true).is_err());
    assert!(grammar::parse_governed_native_registry_layout(&declaration, &VAULT, true).is_err());
    assert!(grammar::parse_shared_history_native_registry_layout(&prior, &VAULT, true).is_err());
    assert!(
        grammar::parse_shared_history_native_registry_layout(&declaration, &VAULT, false).is_err()
    );
    assert!(history::prepare_declaration(&declaration, &VAULT).is_err());
    assert!(history::verify_declaration(&joined, &declaration, &VAULT).is_err());
    assert!(history::verify_declaration(&prior, &declaration, &[0x99; 32]).is_err());

    let descriptor = parsed.shared_history_payload.unwrap();
    let wrong_version =
        catalog_declaration::insert_entry_data(&prior, &VAULT, history::ID, 2, descriptor).unwrap();
    let mut foreign = descriptor.to_vec();
    *foreign.last_mut().unwrap() ^= 1;
    let foreign = catalog_declaration::insert_entry_data(
        &prior,
        &VAULT,
        history::ID,
        history::VERSION,
        &foreign,
    )
    .unwrap();
    for malformed in [&wrong_version, &foreign] {
        assert!(history::verify_declaration(&prior, malformed, &VAULT).is_err());
        assert!(
            grammar::parse_shared_history_native_registry_layout(malformed, &VAULT, true).is_err()
        );
    }
}

#[test]
fn same_vault_rejoin_catalog_adds_only_its_critical_capability() {
    use crate::store::critical_namespaces::{
        rejoin_namespace as rejoin, shared_history_namespace as history,
    };
    let (joined, _) = joined_catalog();
    assert!(rejoin::prepare_declaration(&joined, &VAULT).is_err());
    let prior = history::prepare_declaration(&joined, &VAULT).unwrap();
    let successor = rejoin::prepare_declaration(&prior, &VAULT).unwrap();
    rejoin::verify_declaration(&prior, &successor, &VAULT).unwrap();
    assert!(grammar::parse_rejoin_native_registry_layout(&prior, &VAULT, true).is_err());
    assert!(
        grammar::parse_rejoin_native_registry_layout(&successor, &VAULT, true)
            .unwrap()
            .rejoin_payload
            .is_some()
    );
    // Existing reader still accepts its own original and refuses the new
    // capability before the same joined rows can be activated.
    grammar::parse_shared_history_native_registry_layout(&prior, &VAULT, true).unwrap();
    assert!(
        grammar::parse_shared_history_native_registry_layout(&successor, &VAULT, true).is_err()
    );
    assert!(rejoin::verify_declaration(&joined, &successor, &VAULT).is_err());
    assert!(rejoin::prepare_declaration(&successor, &VAULT).is_err());
    assert!(rejoin::verify_declaration(&prior, &successor, &[0xef; 32]).is_err());
    let parsed = outer::parse(&successor, &VAULT, outer::MAX_ENVELOPE_PLAIN).unwrap();
    let mut changed = parsed.entry(parsed.count - 1).unwrap().payload.to_vec();
    *changed.last_mut().unwrap() ^= 1;
    let malformed =
        catalog_declaration::insert_entry_data(&prior, &VAULT, rejoin::ID, 1, &changed).unwrap();
    assert!(rejoin::verify_declaration(&prior, &malformed, &VAULT).is_err());
    assert!(grammar::parse_rejoin_native_registry_layout(&malformed, &VAULT, true).is_err());
}

#[test]
fn same_vault_rejoin_framed_stage_requires_its_authenticated_selected_parent() {
    use crate::store::critical_namespaces::{rejoin_namespace as rejoin, shared_history_namespace};
    use rand_core::SeedableRng;

    let (joined, _) = joined_catalog();
    let prior = shared_history_namespace::prepare_declaration(&joined, &VAULT).unwrap();
    let key = [0x91; 32];
    let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(0x921);
    let current = crate::store::frame(&catcoms_crypto::seal(&key, &prior, &mut rng).unwrap());
    let candidate = rejoin::prepare_framed_declaration(&key, &current, &VAULT, &mut rng).unwrap();
    rejoin::verify_framed_declaration(&key, &current, &candidate, &VAULT).unwrap();

    let wrong_current =
        crate::store::frame(&catcoms_crypto::seal(&key, &joined, &mut rng).unwrap());
    assert!(rejoin::verify_framed_declaration(&key, &wrong_current, &candidate, &VAULT).is_err());
    assert!(rejoin::verify_framed_declaration(&[0x92; 32], &current, &candidate, &VAULT).is_err());
    assert!(rejoin::verify_framed_declaration(&key, &candidate, &current, &VAULT).is_err());
    let mut corrupt = candidate.to_vec();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(rejoin::verify_framed_declaration(&key, &current, &corrupt, &VAULT).is_err());
    assert!(rejoin::verify_framed_declaration(
        &key,
        &current,
        &candidate[..candidate.len() - 1],
        &VAULT
    )
    .is_err());
}

#[test]
fn shared_history_seed_retains_exact_catalogs_marker_and_pin_index() {
    use crate::store::critical_namespaces::{
        joined_pin::declaration as joined, shared_history_namespace::declaration as history,
    };
    use catcoms_crypto::seal;
    use catcoms_rt::OsCryptoRng;

    let (source, cap) = joined_catalog();
    let layout = CurrentLayoutData {
        vault: cap.vault,
        root: cap.root,
    };
    let key = [0x81; 32];
    let native =
        joined::DeclarationLeaves::from_frames(&[0x82; 41], &[0x83; 41], &[0x84; 41], &[0x85; 41])
            .unwrap();
    let leaves = history::Leaves::from_frames(native, &[0x86; 41]).unwrap();
    let mut rng = OsCryptoRng;
    let original = crate::store::frame(&seal(&key, &source, &mut rng).unwrap());
    let prepared =
        history::PreparedDeclaration::prepare(&key, &original, layout, leaves, &mut rng).unwrap();
    let seed = history::AuthenticatedSeed::open(&key, &prepared.seed).unwrap();
    let opened = seed.validate(&key, layout, leaves).unwrap();
    assert_eq!(opened.original, original.as_slice());
    assert_eq!(opened.candidate, prepared.candidate.as_slice());
    assert_eq!(opened.marker, prepared.marker.as_slice());
    assert!(joined::AuthenticatedSeed::open(&key, &prepared.seed).is_err());
    assert!(
        history::AuthenticatedSeed::open(&key, &prepared.seed[..prepared.seed.len() - 1]).is_err()
    );
    let changed_pin = history::Leaves::from_frames(native, &[0x87; 41]).unwrap();
    assert!(seed.validate(&key, layout, changed_pin).is_err());
    let foreign_layout = CurrentLayoutData {
        root: [0x88; 32],
        ..layout
    };
    assert!(seed.validate(&key, foreign_layout, leaves).is_err());
    assert!(history::PreparedDeclaration::prepare(
        &key,
        &original,
        foreign_layout,
        leaves,
        &mut rng
    )
    .is_err());
    assert!(history::validate_marker(&key, layout, b"foreign descriptor", opened.marker).is_err());
}

#[test]
fn review_hold_namespace_preserves_the_exact_base_and_refuses_id9() {
    use crate::store::critical_namespaces::{
        rejoin_namespace as rejoin, review_hold_namespace as review,
        shared_history_namespace as history,
    };
    let (joined, _) = joined_catalog();
    assert!(review::prepare_declaration(&joined, &VAULT).is_err());
    let prior = history::prepare_declaration(&joined, &VAULT).unwrap();
    let candidate = review::prepare_declaration(&prior, &VAULT).unwrap();
    review::verify_declaration(&prior, &candidate, &VAULT).unwrap();
    assert!(grammar::parse_native_rejoin_startup(&candidate, &VAULT, true).is_err());
    assert!(review::prepare_declaration(&candidate, &VAULT).is_err());
    assert!(review::verify_declaration(&prior, &candidate, &[0x93; 32]).is_err());
    assert!(review::verify_declaration(&joined, &candidate, &VAULT).is_err());

    let parsed = grammar::parse_review_hold_family(&candidate, &VAULT, true).unwrap();
    let descriptor = parsed.review_hold_payload.unwrap();
    let marker = review::marker_data(&[1; 32], descriptor).unwrap();
    assert_ne!(marker, review::marker_data(&[2; 32], descriptor).unwrap());
    assert!(review::marker_data(&[1; 32], &descriptor[..31]).is_err());
    let a2 = rejoin::prepare_declaration(&prior, &VAULT).unwrap();
    assert!(review::prepare_declaration(&a2, &VAULT).is_err());
    let mixed = catalog_declaration::insert_entry_data(
        &a2,
        &VAULT,
        review::ID,
        review::VERSION,
        descriptor,
    )
    .unwrap();
    assert!(grammar::parse_review_hold_family(&mixed, &VAULT, true).is_err());

    let mut changed = descriptor.to_vec();
    changed[0] ^= 1;
    for (version, bytes) in [(2, descriptor), (review::VERSION, changed.as_slice())] {
        let malformed =
            catalog_declaration::insert_entry_data(&prior, &VAULT, review::ID, version, bytes)
                .unwrap();
        assert!(review::verify_declaration(&prior, &malformed, &VAULT).is_err());
    }
}

#[test]
fn founder_review_seed_replays_the_exact_predecessor_and_requires_each_leaf() {
    use crate::store::critical_namespaces::{
        archive_current_namespace_codec::CurrentLayoutData,
        rejoin_namespace,
        review_hold_namespace::declaration::{
            AuthenticatedSeed, FounderLeaves, PreparedDeclaration,
        },
        shared_history_namespace,
    };
    use crate::store::frame;
    use catcoms_crypto::seal;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    let mut rng = ChaCha20Rng::seed_from_u64(4901);
    let key = [91; 32];
    let (joined, _) = joined_catalog();
    let old = shared_history_namespace::prepare_declaration(&joined, &VAULT).unwrap();
    let layout = grammar::parse_shared_history_native_registry_layout(&old, &VAULT, true)
        .unwrap()
        .current_layout
        .unwrap();
    let original = frame(&seal(&key, &old, &mut rng).unwrap());
    let leaves = FounderLeaves {
        row: [1; 32],
        index: [2; 32],
        pins: [3; 32],
        history: [4; 32],
    };
    let prepared = PreparedDeclaration::prepare(&key, &original, layout, leaves, &mut rng).unwrap();
    for end in 0..prepared.seed.len() {
        assert!(AuthenticatedSeed::is_incomplete_scratch(
            &prepared.seed[..end]
        ));
    }
    assert!(!AuthenticatedSeed::is_incomplete_scratch(&prepared.seed));
    assert!(!AuthenticatedSeed::is_incomplete_scratch(b"unknown"));
    let seed = AuthenticatedSeed::open(&key, &prepared.seed).unwrap();
    let recovered = seed.validate(&key, layout, leaves).unwrap();
    assert_eq!(recovered.original, original);
    assert_eq!(recovered.candidate, prepared.candidate.as_slice());
    assert_eq!(recovered.marker, prepared.marker);
    for changed in [
        FounderLeaves {
            row: [101; 32],
            ..leaves
        },
        FounderLeaves {
            index: [102; 32],
            ..leaves
        },
        FounderLeaves {
            pins: [103; 32],
            ..leaves
        },
        FounderLeaves {
            history: [104; 32],
            ..leaves
        },
    ] {
        assert!(seed.validate(&key, layout, changed).is_err());
    }
    let wrong = CurrentLayoutData {
        root: [14; 32],
        ..layout
    };
    assert!(seed.validate(&key, wrong, leaves).is_err());
    assert!(AuthenticatedSeed::open(&[90; 32], &prepared.seed).is_err());
    let mut tainted = prepared.seed;
    *tainted.last_mut().unwrap() ^= 1;
    assert!(!AuthenticatedSeed::is_incomplete_scratch(&tainted));
    assert!(AuthenticatedSeed::open(&key, &tainted).is_err());

    let a2 = rejoin_namespace::prepare_declaration(&old, &VAULT).unwrap();
    let frame = frame(&seal(&key, &a2, &mut rng).unwrap());
    assert!(PreparedDeclaration::prepare(&key, &frame, layout, leaves, &mut rng).is_err());
}
