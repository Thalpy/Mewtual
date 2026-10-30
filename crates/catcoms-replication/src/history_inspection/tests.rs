use super::*;
use automerge::transaction::Transactable;

fn raw(actor: usize, text: &str) -> Vec<u8> {
    let mut doc =
        automerge::AutoCommit::new().with_actor(automerge::ActorId::from(vec![0x23; actor]));
    doc.put(automerge::ROOT, "text", text).unwrap();
    doc.get_last_local_change().unwrap().raw_bytes().to_vec()
}

fn refuse_before_parser(bytes: &[u8]) -> LegacyChangeInspection {
    inspect_with_parser(bytes, |_| -> Result<automerge::Change, ()> {
        panic!("refused raw input must never reach Automerge's parser")
    })
}

fn unsigned(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let low = (value & 127) as u8;
        value >>= 7;
        out.push(low | if value == 0 { 0 } else { 128 });
        if value == 0 {
            break;
        }
    }
}

/// A tiny raw-column fixture with a real checksum. Parser traps prove dangerous column
/// expansion is refused before Automerge is entered, even when the framing is valid.
fn columns(columns: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let mut payload = vec![0, 32];
    payload.extend([0x23; 32]);
    payload.extend([1, 1, 0, 0, 0]);
    unsigned(columns.len() as u64, &mut payload);
    for (spec, data) in columns {
        unsigned(*spec, &mut payload);
        unsigned(data.len() as u64, &mut payload);
    }
    for (_, data) in columns {
        payload.extend(data);
    }
    let mut frame = vec![0x85, 0x6f, 0x4a, 0x83, 0, 0, 0, 0, 1];
    unsigned(payload.len() as u64, &mut frame);
    frame.extend(payload);
    use sha2::{Digest, Sha256};
    let checksum = Sha256::digest(&frame[8..]);
    frame[4..8].copy_from_slice(&checksum[..4]);
    frame
}

#[test]
fn history_inspection_out_of_table_actor_references_refuse_before_expansion() {
    // These are otherwise valid single-op frames: the pinned parser accepts them but its
    // expanded conversion would unwrap missing actor 1 (the table only contains author 0).
    let mut map_key = vec![1, 4];
    map_key.extend(b"text");
    let object = columns(&[
        (0x01, vec![1, 1]),
        (0x02, vec![1, 1]),
        (0x15, map_key.clone()),
        (0x34, vec![1]),
        (0x42, vec![1, 1]),
        (0x56, vec![1, 0]),
        (0x70, vec![1, 0]),
    ]);
    let key = columns(&[
        (0x11, vec![1, 1]),
        (0x13, vec![1, 1]),
        (0x34, vec![1]),
        (0x42, vec![1, 1]),
        (0x56, vec![1, 0]),
        (0x70, vec![1, 0]),
    ]);
    let pred = columns(&[
        (0x15, map_key),
        (0x34, vec![1]),
        (0x42, vec![1, 1]),
        (0x56, vec![1, 0]),
        (0x70, vec![1, 1]),
        (0x71, vec![1, 1]),
        (0x73, vec![1, 1]),
    ]);
    for original in [object, key, pred] {
        assert!(
            automerge::Change::from_bytes(original.clone()).is_ok(),
            "{:?}",
            automerge::Change::from_bytes(original.clone())
        );
        assert!(matches!(
            refuse_before_parser(&original),
            LegacyChangeInspection::Invalid
        ));
    }
}

#[test]
fn history_inspection_checksum_and_nonminimal_length_are_checked_without_rewriting_original() {
    let original = columns(&[
        (0x15, vec![1, 1, b'x']),
        (0x34, vec![1]),
        (0x42, vec![1, 1]),
        (0x56, vec![1, 0]),
        (0x70, vec![1, 0]),
    ]);
    assert!(original[9] < 128);
    let mut nonminimal = original.clone();
    nonminimal[9] |= 128;
    nonminimal.insert(10, 0);
    assert!(matches!(
        inspect_legacy_change(&original),
        LegacyChangeInspection::Decoded(_)
    ));
    assert!(matches!(
        inspect_legacy_change(&nonminimal),
        LegacyChangeInspection::Invalid
    ));
    assert_ne!(original, nonminimal);
}

#[test]
fn history_inspection_reads_legacy_names_without_promoting_them_to_modern_authority() {
    for actor in [1, 16, 32, 256] {
        let original = raw(actor, "original text");
        let LegacyChangeInspection::Decoded(expanded) = inspect_legacy_change(&original) else {
            panic!("bounded historical actor should be inspectable");
        };
        assert!(!expanded.operations.is_empty());
        assert!(format!("{expanded:?}").contains("original text"));
        let parsed = automerge::Change::from_bytes(original).unwrap();
        assert_eq!(
            crate::decode_change_projection(&parsed).is_ok(),
            actor == 32,
            "existing authenticated projection must retain its strict actor rule"
        );
    }
    assert!(matches!(
        refuse_before_parser(&raw(257, "not a signing identity")),
        LegacyChangeInspection::Unsupported
    ));
}

#[test]
fn history_inspection_keeps_each_original_edit_and_deletion_change_readable() {
    let mut doc = automerge::AutoCommit::new().with_actor(automerge::ActorId::from(vec![0x23; 16]));
    let mut originals = Vec::new();
    doc.put(automerge::ROOT, "text", "first version").unwrap();
    originals.push(doc.get_last_local_change().unwrap().raw_bytes().to_vec());
    doc.put(automerge::ROOT, "text", "edited version").unwrap();
    originals.push(doc.get_last_local_change().unwrap().raw_bytes().to_vec());
    doc.delete(automerge::ROOT, "text").unwrap();
    originals.push(doc.get_last_local_change().unwrap().raw_bytes().to_vec());
    let projections: Vec<_> = originals
        .iter()
        .map(|bytes| {
            let LegacyChangeInspection::Decoded(change) = inspect_legacy_change(bytes) else {
                panic!("original change disappeared");
            };
            change
        })
        .collect();
    assert!(format!("{:?}", projections[0]).contains("first version"));
    assert!(!format!("{:?}", projections[0]).contains("edited version"));
    assert!(format!("{:?}", projections[1]).contains("edited version"));
    assert!(!projections[2].operations.is_empty());
    assert!(!projections[1].deps.is_empty() && !projections[2].deps.is_empty());
}

#[test]
fn history_inspection_large_rle_pred_and_string_claims_never_reach_parser() {
    let huge_run = columns(&[(0x42, vec![0x80, 0x80, 0x80, 0x80, 0x08, 1])]);
    assert!(matches!(
        refuse_before_parser(&huge_run),
        LegacyChangeInspection::TooLarge
    ));
    let mut preds = vec![1];
    unsigned(4097, &mut preds);
    assert!(matches!(
        refuse_before_parser(&columns(&[(0x42, vec![1, 1]), (0x70, preds)])),
        LegacyChangeInspection::TooLarge
    ));
    let mut text = vec![1];
    unsigned(1 << 30, &mut text);
    assert!(matches!(
        refuse_before_parser(&columns(&[(0x15, text), (0x42, vec![1, 1])])),
        LegacyChangeInspection::Invalid
    ));
}

#[test]
fn history_inspection_raw_and_action_caps_precede_parsing() {
    assert!(matches!(
        refuse_before_parser(&vec![0; MAX_INSPECTED_CHANGE_BYTES + 1]),
        LegacyChangeInspection::TooLarge
    ));
    let mut doc = automerge::AutoCommit::new().with_actor(automerge::ActorId::from(vec![0x23; 32]));
    for n in 0..513 {
        doc.put(automerge::ROOT, format!("key{n}"), n as u64)
            .unwrap();
    }
    let original = doc.get_last_local_change().unwrap().raw_bytes().to_vec();
    assert!(original.len() <= MAX_INSPECTED_CHANGE_BYTES);
    assert!(matches!(
        refuse_before_parser(&original),
        LegacyChangeInspection::TooLarge
    ));
}

#[test]
fn history_inspection_compressed_input_is_explicitly_unsupported_and_malformed_is_not_decoded() {
    let mut doc = automerge::AutoCommit::new().with_actor(automerge::ActorId::from(vec![0x23; 16]));
    doc.put(automerge::ROOT, "text", "x".repeat(20_000))
        .unwrap();
    let mut change = doc.get_last_local_change().unwrap();
    let compressed = change.bytes().into_owned();
    assert_eq!(compressed[8], 2);
    assert!(matches!(
        refuse_before_parser(&compressed),
        LegacyChangeInspection::Unsupported
    ));
    assert!(matches!(
        refuse_before_parser(b"not a change"),
        LegacyChangeInspection::Invalid
    ));
    let mut corrupt = raw(16, "original");
    corrupt[4] ^= 1;
    assert!(matches!(
        inspect_legacy_change(&corrupt),
        LegacyChangeInspection::Invalid
    ));
}
