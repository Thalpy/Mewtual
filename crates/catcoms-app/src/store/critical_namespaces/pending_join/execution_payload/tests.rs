//! Syntax-only DATA fixtures: the fake encrypted source is intentionally not authentic and
//! cannot supply an App preparation/task/grant. Actual mounted/network acceptance is separate.
//! All tests authored UNRUN; only the explicit private execution policy selects descriptor-v2.
use super::*;
use catcoms_wire::Encoder;

const DOMAIN: [u8; 32] = [0x51; 32];

fn original(n: u8, active: bool, root_kind: u8) -> (Vec<u8>, execution::SourceIdentity) {
    let mut immutable = Encoder::new();
    immutable.put_str(codec::IMMUTABLE_TAG).unwrap();
    immutable.put_bytes(&DOMAIN).unwrap();
    immutable.put_bytes(&[n; 32]).unwrap();
    immutable.put_u8(1);
    immutable.put_u8(root_kind);
    for field in [
        &[n][..],
        &[n; 32],
        &[2; 32],
        &[3; 32],
        &[n; 16],
        &[4; 32],
        &[5; 32],
        &[6; 32],
    ] {
        immutable.put_bytes(field).unwrap();
    }
    immutable.put_bytes(&[0]).unwrap(); // absent basis DATA, never actual admission
    let immutable = immutable.finish();
    let identity = execution::SourceIdentity {
        binding: codec::binding_digest(&immutable),
        request: [5; 32],
        package: [6; 32],
    };
    let mut row = Encoder::new();
    row.put_bytes(&[n; 32]).unwrap();
    row.put_u64(1);
    row.put_u8(if active { 0 } else { 1 });
    row.put_u8(1);
    row.put_bytes(&immutable).unwrap();
    row.put_bytes(&[0; 40]).unwrap();
    (row.finish(), identity)
}
fn row(n: u8, active: bool, root_kind: u8) -> Vec<u8> {
    let (original, identity) = original(n, active, root_kind);
    let state = execution::State::not_granted();
    let length = execution::encoded_len(&state, None, &identity, 1).unwrap();
    let mut record = Encoder::with_capacity(length);
    execution::encode_into(&state, None, &identity, 1, &mut record).unwrap();
    let mut row = Encoder::new();
    row.put_bytes(&original).unwrap();
    row.put_bytes(&record.finish()).unwrap();
    row.finish()
}
fn payload(rows: &[Vec<u8>]) -> Vec<u8> {
    let mut output = Encoder::new();
    output.put_str(TAG).unwrap();
    output.put_u32(rows.len() as u32);
    for row in rows {
        output.put_bytes(row).unwrap();
    }
    output.finish()
}

#[test]
fn five_not_granted_data_rows_allow_all_paused_and_do_not_enable_default_v1_reader() {
    let bytes = payload(&(1..=5).map(|n| row(n, false, 1)).collect::<Vec<_>>());
    let (rows, count) = parse(&bytes, &DOMAIN).unwrap();
    assert_eq!(count, 5);
    assert!(rows
        .iter()
        .flatten()
        .all(|row| row.metadata.execution == Some(execution::State::not_granted())));
    let entry = crate::store::critical_namespaces::catalog_envelope::Entry {
        id: codec::JOIN_ID,
        version: VERSION,
        payload: &bytes,
    };
    assert!(
        codec::parse_join_entry(entry, &DOMAIN).is_err(),
        "DATA decoder does not enable configured support"
    );
}

#[test]
fn execution_payload_rejects_count_order_duplicate_active_trailing_and_other_root_kind() {
    for rows in [
        vec![],
        vec![row(2, false, 1), row(1, false, 1)],
        vec![row(1, false, 1), row(1, false, 1)],
        vec![row(1, true, 1), row(2, true, 1)],
        vec![row(1, false, 2)],
    ] {
        assert!(parse(&payload(&rows), &DOMAIN).is_err());
    }
    let mut bytes = payload(&[row(1, true, 1)]);
    bytes.push(0);
    assert!(parse(&bytes, &DOMAIN).is_err());
    assert!(parse(&payload(&[row(1, true, 1)]), &[9; 32]).is_err());
    let sixth = payload(&(1..=6).map(|n| row(n, false, 1)).collect::<Vec<_>>());
    assert!(parse(&sixth, &DOMAIN).is_err());
}

#[test]
fn complete_source_and_execution_nested_framing_precedes_any_output_or_authority() {
    let bytes = payload(&[row(1, true, 1)]);
    for length in 0..bytes.len() {
        assert!(parse(&bytes[..length], &DOMAIN).is_err());
    }
    let mut huge = Encoder::new();
    huge.put_str(TAG).unwrap();
    huge.put_u32(u32::MAX);
    assert!(parse(&huge.finish(), &DOMAIN).is_err());
    let mut row = row(1, true, 1);
    row.extend_from_slice(&[0]);
    assert!(parse(&payload(&[row]), &DOMAIN).is_err());
}

#[test]
fn nested_encoder_preserves_exact_original_v1_and_execution_frames_without_temporary_record_vec() {
    let (original, _) = original(1, true, 1);
    let parsed_original = codec::parse_original_row(&original, &DOMAIN).unwrap();
    let mut encoded_original = Encoder::new();
    codec::encode_original_row(
        &mut encoded_original,
        &parsed_original.metadata,
        parsed_original.immutable,
        parsed_original.encrypted,
    )
    .unwrap();
    let bytes = encoded_original.finish();
    let mut reader = Decoder::new(&bytes);
    assert_eq!(reader.get_bytes().unwrap(), original.as_slice());
    reader.finish().unwrap();

    let expected = row(1, true, 1);
    let bytes = payload(std::slice::from_ref(&expected));
    let (rows, _) = parse(&bytes, &DOMAIN).unwrap();
    let row = rows[0].as_ref().unwrap();
    let length = encoded_row_len(
        &row.metadata,
        row.immutable,
        row.encrypted,
        row.execution_input.as_ref(),
    )
    .unwrap();
    let mut output = Encoder::with_capacity(4 + length);
    encode_row(
        &mut output,
        &row.metadata,
        row.immutable,
        row.encrypted,
        row.execution_input.as_ref(),
    )
    .unwrap();
    let encoded = output.finish();
    assert_eq!(encoded.len(), 4 + length);
    let mut reader = Decoder::new(&encoded);
    assert_eq!(reader.get_bytes().unwrap(), expected.as_slice());
    reader.finish().unwrap();
}

fn summary(
    metadata: &[codec::Metadata],
    version: u32,
    generation: u64,
    parent: Option<[u8; 32]>,
    digest: [u8; 32],
) -> codec::Summary {
    codec::Summary {
        generation,
        parent,
        digest,
        framed: [0; 32],
        rows: std::array::from_fn(|index| metadata.get(index).cloned()),
        count: metadata.len(),
        archive: None,
        join_version: Some(version),
        current_layout: None,
        current_payload_digest: None,
    }
}
fn original_metadata(n: u8, active: bool) -> codec::Metadata {
    let (bytes, _) = original(n, active, 1);
    codec::parse_original_row(&bytes, &DOMAIN).unwrap().metadata
}

#[test]
fn whole_parent_migration_is_all_source_rows_unchanged_and_gives_no_grants_or_debits() {
    let first = original_metadata(1, true);
    let second = original_metadata(2, false);
    let old = summary(
        &[first.clone(), second.clone()],
        codec::JOIN_VERSION,
        2,
        None,
        [1; 32],
    );
    let mut first = first;
    let mut second = second;
    first.execution = Some(execution::State::not_granted());
    second.execution = Some(execution::State::not_granted());
    let next = summary(
        &[first.clone(), second.clone()],
        VERSION,
        3,
        Some(old.digest),
        [2; 32],
    );
    codec::successor(&old, &next).unwrap();
    let mut partial = next.clone();
    partial.rows[1].as_mut().unwrap().execution = None;
    assert!(codec::successor(&old, &partial).is_err());
    let mut grant = next.clone();
    let state = grant.rows[0].as_mut().unwrap().execution.as_mut().unwrap();
    state.grant = 1;
    state.limit = 64;
    assert!(
        codec::successor(&old, &grant).is_err(),
        "migration cannot grant units as a side effect"
    );
    let mut source = next.clone();
    source.rows[0].as_mut().unwrap().ciphertext[0] ^= 1;
    assert!(codec::successor(&old, &source).is_err());
    let mut wrong_parent = next;
    wrong_parent.parent = Some([8; 32]);
    assert!(codec::successor(&old, &wrong_parent).is_err());
}

#[test]
fn exact_one_execution_event_or_schedule_change_and_immutable_source_preservation() {
    let mut first = original_metadata(1, true);
    let mut second = original_metadata(2, false);
    first.execution = Some(execution::State::not_granted());
    second.execution = Some(execution::State::not_granted());
    let old = summary(&[first.clone(), second.clone()], VERSION, 3, None, [3; 32]);
    let mut granted = first.clone();
    let state = granted.execution.as_mut().unwrap();
    state.grant = 1;
    state.limit = 64;
    let next = summary(
        &[granted.clone(), second.clone()],
        VERSION,
        4,
        Some(old.digest),
        [4; 32],
    );
    codec::successor(&old, &next).unwrap();
    let mut two = next.clone();
    let state = two.rows[1].as_mut().unwrap().execution.as_mut().unwrap();
    state.grant = 1;
    state.limit = 64;
    assert!(codec::successor(&old, &two).is_err());
    let mut altered = next.clone();
    altered.rows[0].as_mut().unwrap().request[0] ^= 1;
    assert!(
        codec::successor(&old, &altered).is_err(),
        "matching hash labels cannot hide changed summary identity"
    );
    let mut schedule = first;
    schedule.revision += 1;
    schedule.scheduling = codec::Scheduling::Paused;
    let scheduled = summary(&[schedule, second], VERSION, 4, Some(old.digest), [5; 32]);
    codec::successor(&old, &scheduled).unwrap();
    assert!(
        codec::successor(
            &next,
            &summary(
                &[granted],
                codec::JOIN_VERSION,
                5,
                Some(next.digest),
                [6; 32]
            )
        )
        .is_err(),
        "no downgrade or row removal"
    );
}

#[test]
fn regrant_uses_new_schedule_incarnation_without_toggling_active_and_new_source_is_not_granted() {
    let mut first = original_metadata(1, true);
    first.execution = Some(execution::State {
        grant: 1,
        limit: 64,
        spent: 0,
        ordinal: 0,
        arm: None,
        input: None,
    });
    let old = summary(&[first.clone()], VERSION, 4, None, [4; 32]);
    let mut next = first.clone();
    next.execution.as_mut().unwrap().grant = 2;
    let wrong = summary(&[next.clone()], VERSION, 5, Some(old.digest), [5; 32]);
    assert!(codec::successor(&old, &wrong).is_err());
    next.revision += 1;
    codec::successor(
        &old,
        &summary(&[next], VERSION, 5, Some(old.digest), [5; 32]),
    )
    .unwrap();
    // New sources still receive no grant from an add, and existing operational fields stay exact.
    first.scheduling = codec::Scheduling::Paused;
    let old = summary(&[first.clone()], VERSION, 6, None, [6; 32]);
    let mut new = original_metadata(2, true);
    new.execution = Some(execution::State::not_granted());
    let next = summary(
        &[first.clone(), new.clone()],
        VERSION,
        7,
        Some(old.digest),
        [7; 32],
    );
    codec::successor(&old, &next).unwrap();
    new.execution = first.execution;
    assert!(codec::successor(
        &old,
        &summary(&[first, new], VERSION, 7, Some(old.digest), [7; 32])
    )
    .is_err());
}
