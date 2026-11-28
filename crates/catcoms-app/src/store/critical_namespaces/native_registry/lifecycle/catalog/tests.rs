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
fn installed_rejoin_catalog_binding_requires_the_exact_encrypted_birth_frame() {
    use crate::store::critical_namespaces::{
        rejoin_namespace as rejoin, review_receive_namespace as review,
        shared_history_namespace as history,
    };
    use rand_core::SeedableRng;

    let (joined, _) = joined_catalog();
    let shared = history::prepare_declaration(&joined, &VAULT).unwrap();
    let initial_review = review::prepare_declaration(&shared, &VAULT).unwrap();
    let receipt = grammar::parse_review_receive_family(&initial_review, &VAULT, true).unwrap();
    let descriptor = receipt.review_receive_payload.unwrap();
    let born = rejoin::prepare_declaration(&shared, &VAULT).unwrap();
    let pre_review = rejoin::prepare_declaration(&initial_review, &VAULT).unwrap();
    let generation = outer::parse(&born, &VAULT, outer::MAX_ENVELOPE_PLAIN)
        .unwrap()
        .generation;
    let key = [0x91; 32];
    let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(0x921);
    let mut seal =
        |plain: &[u8]| crate::store::frame(&catcoms_crypto::seal(&key, plain, &mut rng).unwrap());
    let original = seal(&born);
    let resealed = seal(&born);
    let old_review = seal(&pre_review);
    for direct in [&original, &old_review] {
        let exact = *blake3::hash(direct).as_bytes();
        assert!(matches!(
            rejoin::inspect_installed_catalog_binding(&key, direct, &VAULT).unwrap(),
            rejoin::InstalledCatalogBindingData::Direct(digest) if digest == exact
        ));
        assert_eq!(
            rejoin::selected_frame_digest(&key, direct, &VAULT).unwrap(),
            exact
        );
    }
    let child = |generation, descriptor: &[u8]| {
        let mut data = Encoder::with_capacity(200);
        data.put_bytes(b"catcoms/chat-v2/review-receiver-rejoin-parent/v1\0")
            .unwrap();
        data.put_bytes(descriptor).unwrap();
        data.put_u64(generation);
        data.put_bytes(blake3::hash(&original).as_bytes()).unwrap();
        catalog_declaration::insert_entry_data(&born, &VAULT, review::ID, 2, &data.finish())
            .unwrap()
    };
    let plain = child(generation, descriptor);
    let extended = seal(&plain);
    let rejoin::InstalledCatalogBindingData::ReviewSuccessor(parent) =
        rejoin::inspect_installed_catalog_binding(&key, &extended, &VAULT).unwrap()
    else {
        panic!("linked ID12 must be recognized only by installed replay");
    };
    assert_eq!(
        parent
            .verify_installed_parent(&key, &original, &extended, &VAULT)
            .unwrap(),
        *blake3::hash(&original).as_bytes()
    );
    assert!(rejoin::selected_frame_digest(&key, &extended, &VAULT).is_err());
    assert!(grammar::parse_review_receive_family(&plain, &VAULT, true).is_err());
    assert!(parent
        .verify_installed_parent(&key, &resealed, &extended, &VAULT)
        .is_err());
    assert!(parent
        .verify_installed_parent(&key, &old_review, &extended, &VAULT)
        .is_err());
    assert!(parent
        .verify_installed_parent(&key, &original, &extended, &[0xee; 32])
        .is_err());
    assert!(parent
        .verify_installed_parent(&key, &original[..original.len() - 1], &extended, &VAULT)
        .is_err());
    let wrong_generation = seal(&child(generation - 1, descriptor));
    assert!(rejoin::inspect_installed_catalog_binding(&key, &wrong_generation, &VAULT).is_err());
    let invalid_descriptor = seal(&child(generation, &[0x44; 32]));
    let rejoin::InstalledCatalogBindingData::ReviewSuccessor(invalid) =
        rejoin::inspect_installed_catalog_binding(&key, &invalid_descriptor, &VAULT).unwrap()
    else {
        panic!("descriptor validation is separate from link parsing");
    };
    assert!(invalid
        .verify_installed_parent(&key, &original, &invalid_descriptor, &VAULT)
        .is_err());
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
fn decision_namespace_is_an_explicit_successor_of_the_original_hold_catalog() {
    use crate::store::critical_namespaces::{
        review_decision_namespace as decisions, review_hold_namespace as holds,
        shared_history_namespace as history,
    };
    let (joined, _) = joined_catalog();
    let shared = history::prepare_declaration(&joined, &VAULT).unwrap();
    assert!(decisions::prepare_declaration(&shared, &VAULT).is_err());
    let held = holds::prepare_declaration(&shared, &VAULT).unwrap();
    let selected = decisions::prepare_declaration(&held, &VAULT).unwrap();
    decisions::verify_declaration(&held, &selected, &VAULT).unwrap();
    assert!(grammar::parse_review_hold_family(&selected, &VAULT, true).is_err());
    assert!(grammar::parse_native_rejoin_startup(&selected, &VAULT, true).is_err());
    assert!(grammar::parse_review_decision_family(&held, &VAULT, true).is_err());
    assert!(decisions::prepare_declaration(&selected, &VAULT).is_err());

    let parsed = grammar::parse_review_decision_family(&selected, &VAULT, true).unwrap();
    let descriptor = parsed.review_decision_payload.unwrap();
    assert_eq!(
        parsed.review_hold_payload,
        grammar::parse_review_hold_family(&held, &VAULT, true)
            .unwrap()
            .review_hold_payload,
    );
    let key = [11; 32];
    assert_ne!(
        decisions::marker_data(&key, descriptor).unwrap(),
        holds::marker_data(&key, descriptor).unwrap(),
    );
    assert!(decisions::verify_declaration(&held, &selected, &[1; 32]).is_err());
    let mut altered = descriptor.to_vec();
    altered[0] ^= 1;
    for (version, payload) in [(2, descriptor), (decisions::VERSION, altered.as_slice())] {
        let malformed =
            catalog_declaration::insert_entry_data(&held, &VAULT, decisions::ID, version, payload)
                .unwrap();
        assert!(grammar::parse_review_decision_family(&malformed, &VAULT, true).is_err());
    }
}

#[test]
fn received_review_namespace_precedes_rejoin_and_keeps_the_legacy_grammar_closed() {
    use crate::store::critical_namespaces::{
        rejoin_namespace as rejoin, review_receive_namespace as review,
        shared_history_namespace as history,
    };
    let (joined, _) = joined_catalog();
    let shared = history::prepare_declaration(&joined, &VAULT).unwrap();
    let recipient = review::prepare_declaration(&shared, &VAULT).unwrap();
    review::verify_declaration(&shared, &recipient, &VAULT).unwrap();
    assert!(grammar::parse_native_review_startup(&recipient, &VAULT, true).is_err());
    assert!(grammar::parse_review_receive_family(&shared, &VAULT, true).is_err());
    assert!(review::prepare_declaration(&recipient, &VAULT).is_err());
    let parsed = grammar::parse_review_receive_family(&recipient, &VAULT, true).unwrap();
    let review_descriptor = parsed.review_receive_payload.unwrap();
    assert_ne!(
        review::marker_data(&[1; 32], review_descriptor).unwrap(),
        review::marker_data(&[2; 32], review_descriptor).unwrap()
    );
    let mut altered = review_descriptor.to_vec();
    altered[0] ^= 1;
    let forged = catalog_declaration::insert_entry_data(
        &shared,
        &VAULT,
        review::ID,
        review::VERSION,
        &altered,
    )
    .unwrap();
    assert!(grammar::parse_review_receive_family(&forged, &VAULT, true).is_err());

    let prior_rejoin = rejoin::prepare_declaration(&shared, &VAULT).unwrap();
    assert!(review::prepare_declaration(&prior_rejoin, &VAULT).is_err());
    let prior = grammar::parse_rejoin_native_registry_layout(&prior_rejoin, &VAULT, true).unwrap();
    let rejoin_descriptor = prior.rejoin_payload.unwrap();
    // ID9's descriptor independently binds the same original base.
    let combined = rejoin::prepare_declaration(&recipient, &VAULT).unwrap();
    rejoin::verify_declaration(&recipient, &combined, &VAULT).unwrap();
    let next = grammar::parse_review_receive_family(&combined, &VAULT, true).unwrap();
    assert_eq!(next.review_receive_payload, Some(review_descriptor));
    assert_eq!(next.rejoin_payload, Some(rejoin_descriptor));
    assert!(rejoin::prepare_declaration(&combined, &VAULT).is_err());
    assert!(rejoin::verify_declaration(&recipient, &prior_rejoin, &VAULT).is_err());
    assert!(grammar::parse_rejoin_native_registry_layout(&combined, &VAULT, true).is_err());

    use catcoms_crypto::seal;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    let mut rng = ChaCha20Rng::seed_from_u64(4912);
    let key = [97; 32];
    let original = crate::store::frame(&seal(&key, &recipient, &mut rng).unwrap());
    let successor = rejoin::prepare_framed_declaration(&key, &original, &VAULT, &mut rng).unwrap();
    rejoin::verify_framed_declaration(&key, &original, &successor, &VAULT).unwrap();
    assert_eq!(
        rejoin::selected_frame_digest(&key, &successor, &VAULT).unwrap(),
        *blake3::hash(&successor).as_bytes()
    );
    assert!(rejoin::selected_frame_digest(&key, &original, &VAULT).is_err());
}

#[test]
fn fresh_native_review_layout_accepts_exact_review_successors_without_cold_authority() {
    use crate::store::critical_namespaces::{
        native_registry::{
            lifecycle::{admission::Handler, preparation::Policy},
            preparation::NativeRegistryHandler,
        },
        rejoin_namespace as rejoin, review_decision_namespace as decisions,
        review_hold_namespace as holds, review_receive_namespace as recipients,
        shared_history_namespace::{self as history, Opening},
    };
    use crate::store::CriticalCatalogReadPool;

    let (joined, _) = joined_catalog();
    let shared = history::prepare_declaration(&joined, &VAULT).unwrap();
    let hold = holds::prepare_declaration(&shared, &VAULT).unwrap();
    let decision = decisions::prepare_declaration(&hold, &VAULT).unwrap();
    let recipient = recipients::prepare_declaration(&shared, &VAULT).unwrap();
    let rejoined = rejoin::prepare_declaration(&recipient, &VAULT).unwrap();
    let policy = Policy::new(
        CriticalCatalogReadPool::new(
            crate::store::critical_namespaces::native_registry::lifecycle::preparation::PREPARATION_FEE,
        ).unwrap(),
        Policy::required_persistent_capacity(),
    ).unwrap();
    let original = Handler::from_reserved(&policy, policy.reserve().unwrap()).unwrap();
    let before = NativeRegistryHandler::GovernedLifecycle(
        original
            .clone()
            .with_shared_history_layout(Opening::Required),
    );
    let warm = NativeRegistryHandler::GovernedLifecycle(original.with_warm_review_layout());
    assert!(!warm.accepts_review_recovery());
    assert!(before.parse_layout(&shared, &VAULT, true).is_ok());
    assert!(warm.parse_layout(&shared, &VAULT, true).is_ok());
    for review in [&hold, &decision, &recipient] {
        assert!(before.parse_layout(review, &VAULT, true).is_err());
        assert!(warm.parse_layout(review, &VAULT, true).is_ok());
        assert!(warm.parse_layout(review, &[33; 32], true).is_err());
    }
    assert!(warm.parse_layout(&rejoined, &VAULT, true).is_err());
}

#[test]
fn founder_review_seed_replays_the_exact_predecessor_and_requires_each_leaf() {
    use crate::store::critical_namespaces::{
        archive_current_namespace_codec::CurrentLayoutData,
        rejoin_namespace,
        review_hold_namespace::declaration::{
            AuthenticatedSeed, FounderLeaves, PreparedDeclaration, ReviewDeclarationKind,
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
    let kind = ReviewDeclarationKind::Hold;
    let prepared =
        PreparedDeclaration::prepare(kind, &key, &original, layout, leaves, &mut rng).unwrap();
    for end in 0..prepared.seed.len() {
        assert!(AuthenticatedSeed::is_incomplete_scratch(
            kind,
            &prepared.seed[..end]
        ));
    }
    assert!(!AuthenticatedSeed::is_incomplete_scratch(
        kind,
        &prepared.seed
    ));
    assert!(!AuthenticatedSeed::is_incomplete_scratch(kind, b"unknown"));
    let seed = AuthenticatedSeed::open(kind, &key, &prepared.seed).unwrap();
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
    assert!(AuthenticatedSeed::open(kind, &[90; 32], &prepared.seed).is_err());
    let decision_kind = ReviewDeclarationKind::Decision;
    assert!(AuthenticatedSeed::open(decision_kind, &key, &prepared.seed).is_err());
    assert!(
        PreparedDeclaration::prepare(decision_kind, &key, &original, layout, leaves, &mut rng)
            .is_err()
    );
    let decision = PreparedDeclaration::prepare(
        decision_kind,
        &key,
        &prepared.candidate,
        layout,
        leaves,
        &mut rng,
    )
    .unwrap();
    assert!(AuthenticatedSeed::open(kind, &key, &decision.seed).is_err());
    let decision_seed = AuthenticatedSeed::open(decision_kind, &key, &decision.seed).unwrap();
    let next = decision_seed.validate(&key, layout, leaves).unwrap();
    assert_eq!(next.original, recovered.candidate);
    assert_eq!(next.candidate, decision.candidate.as_slice());
    assert_ne!(next.marker, recovered.marker);
    let mut tainted = prepared.seed;
    *tainted.last_mut().unwrap() ^= 1;
    assert!(!AuthenticatedSeed::is_incomplete_scratch(kind, &tainted));
    assert!(AuthenticatedSeed::open(kind, &key, &tainted).is_err());

    let a2 = rejoin_namespace::prepare_declaration(&old, &VAULT).unwrap();
    let frame = frame(&seal(&key, &a2, &mut rng).unwrap());
    assert!(PreparedDeclaration::prepare(kind, &key, &frame, layout, leaves, &mut rng).is_err());
}
