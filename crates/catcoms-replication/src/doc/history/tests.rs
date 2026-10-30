use super::*;
use automerge::transaction::Transactable;
use automerge::{ActorId, AutoCommit, ROOT};
use catcoms_mls::{MlsDevice, ServerGroup};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn document(device: &MlsDevice) -> EncryptedDoc {
    EncryptedDoc::new(DocType::Channel, 7, &device.device_id())
}

fn original(device: &MlsDevice, graph: &mut AutoCommit, text: &str) -> crate::SignedOp {
    graph.put(ROOT, "text", text).unwrap();
    graph.commit();
    crate::SignedOp::sign(
        device,
        DocType::Channel,
        7,
        graph.get_last_local_change().unwrap().raw_bytes().to_vec(),
    )
    .unwrap()
}

fn writer(device: &MlsDevice) -> AutoCommit {
    AutoCommit::new().with_actor(ActorId::from(device.device_id().as_bytes().to_vec()))
}

fn apply(doc: &mut EncryptedDoc, op: crate::SignedOp) {
    // The observer tests exercise the real post-decryption signature/actor/graph path.
    // MLS transport/admission is deliberately not inferred from this read-only fixture.
    doc.apply_signed_tracked_observed(op, |_, _| {}).unwrap();
}

fn finish(index: &mut SignedHistoryIndex, doc: &EncryptedDoc) {
    for _ in 0..1000 {
        let work = index.advance(doc).unwrap();
        assert!(work.frame_bytes <= HISTORY_HASH_BYTES_PER_TURN);
        assert!(work.signature_bytes <= HISTORY_SIGNATURE_BYTES_PER_TURN);
        assert!(work.signature_checks <= HISTORY_SIGNATURES_PER_TURN);
        assert!(work.source_records_completed <= HISTORY_RECORDS_PER_TURN);
        if work.complete_signed_prefix {
            return;
        }
    }
    panic!("finite retained originals never completed");
}

#[test]
fn signed_history_keeps_original_create_edits_and_deletion_without_reading_live_messages() {
    let device = MlsDevice::generate().unwrap();
    let mut doc = document(&device);
    let mut graph = writer(&device);
    apply(&mut doc, original(&device, &mut graph, "original"));
    apply(&mut doc, original(&device, &mut graph, "edited"));
    graph.delete(ROOT, "text").unwrap();
    graph.commit();
    apply(
        &mut doc,
        crate::SignedOp::sign(
            &device,
            DocType::Channel,
            7,
            graph.get_last_local_change().unwrap().raw_bytes().to_vec(),
        )
        .unwrap(),
    );
    let mut index = doc.signed_history_index(3).unwrap();
    assert_eq!(index.page(&doc, None, 32), Err(SignedHistoryError::Busy));
    finish(&mut index, &doc);
    let rows = index.page(&doc, None, 32).unwrap();
    assert_eq!(rows.len(), 3);
    for row in rows {
        assert!(row.scope_matches);
        assert_eq!(row.signature, InspectionSignature::Valid);
        let bytes = index
            .original_chunk(&doc, &row.op_hash, 0, MAX_INSPECTION_FRAME_CHUNK)
            .unwrap();
        assert_eq!(bytes, doc.log[row.position].encode());
        assert_eq!(
            index
                .original_chunk(&doc, &row.op_hash, bytes.len(), 1)
                .unwrap(),
            Vec::<u8>::new()
        );
        assert!(matches!(
            index.inspect_original_change(&doc, &row.op_hash).unwrap(),
            LegacyChangeInspection::Decoded(_)
        ));
    }
}

#[test]
fn signed_history_captured_prefix_finishes_while_fifty_writers_append_between_every_visit() {
    let devices: Vec<_> = (0..50).map(|_| MlsDevice::generate().unwrap()).collect();
    let mut graphs: Vec<_> = devices.iter().map(writer).collect();
    let mut doc = document(&devices[0]);
    for n in 0..130 {
        let w = n % 50;
        apply(
            &mut doc,
            original(&devices[w], &mut graphs[w], &format!("before {n}")),
        );
    }
    let expected: Vec<_> = doc.log.iter().map(crate::SignedOp::hash).collect();
    let mut index = doc.signed_history_index(130).unwrap();
    let mut visits = 0;
    loop {
        visits += 1;
        assert!(visits < 20, "appends restarted the captured prefix");
        let work = index.advance(&doc).unwrap();
        for w in 0..50 {
            apply(
                &mut doc,
                original(&devices[w], &mut graphs[w], &format!("tail {visits}")),
            );
        }
        if work.complete_signed_prefix {
            break;
        }
    }
    assert_eq!(index.captured_record_count(), 130);
    let mut actual = Vec::new();
    let mut after = None;
    loop {
        let page = index.page(&doc, after, 17).unwrap();
        if page.is_empty() {
            break;
        }
        after = page.last().map(|row| row.op_hash);
        actual.extend(page.into_iter().map(|row| row.op_hash));
    }
    let mut expected = expected;
    expected.sort_unstable();
    assert_eq!(actual, expected);
    assert!(doc.op_count() > actual.len());
}

#[test]
fn signed_history_same_count_restore_cannot_adopt_an_old_page_or_original_reference() {
    let device = MlsDevice::generate().unwrap();
    let mut doc = document(&device);
    apply(
        &mut doc,
        original(&device, &mut writer(&device), "retained"),
    );
    let mut index = doc.signed_history_index(1).unwrap();
    finish(&mut index, &doc);
    let hash = index.page(&doc, None, 1).unwrap()[0].op_hash;
    let restored = EncryptedDoc::restore(&doc.snapshot().unwrap()).unwrap();
    assert_eq!(index.advance(&restored), Err(SignedHistoryError::Stale));
    assert_eq!(
        index.page(&restored, None, 0),
        Err(SignedHistoryError::Stale)
    );
    assert_eq!(
        index.original_chunk(&restored, &hash, usize::MAX, 0),
        Err(SignedHistoryError::Stale)
    );
}

#[test]
fn signed_history_restored_bad_signature_and_foreign_scope_remain_exact_readable_evidence() {
    let device = MlsDevice::generate().unwrap();
    let mut doc = document(&device);
    let raw = original(&device, &mut writer(&device), "private original");
    apply(&mut doc, raw.clone());
    let mut invalid = raw.clone();
    invalid.signature[0] ^= 1;
    doc.log.push(invalid);
    doc.log
        .push(crate::SignedOp::sign(&device, DocType::Channel, 9, raw.delta).unwrap());
    let restored = EncryptedDoc::restore(&doc.snapshot().unwrap()).unwrap();
    let mut index = restored.signed_history_index(3).unwrap();
    finish(&mut index, &restored);
    let rows = index.page(&restored, None, 32).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter()
            .filter(|row| row.signature == InspectionSignature::Invalid)
            .count(),
        1
    );
    assert_eq!(rows.iter().filter(|row| !row.scope_matches).count(), 1);
    for row in rows {
        assert_eq!(
            index
                .original_chunk(&restored, &row.op_hash, 0, MAX_INSPECTION_FRAME_CHUNK)
                .unwrap(),
            restored.log[row.position].encode()
        );
    }
}

#[test]
fn signed_history_large_original_hashing_resumes_and_signature_limit_is_terminal() {
    let device = MlsDevice::generate().unwrap();
    let mut doc = document(&device);
    let mut raw = original(&device, &mut writer(&device), "initial");
    // Historical restore does not authenticate log entries. A huge retained original must
    // terminate honestly, without attempting its whole preimage or reconstructing its change.
    raw.delta = vec![42; 2 * 1024 * 1024];
    doc.log.push(raw);
    let mut index = doc.signed_history_index(1).unwrap();
    assert!(!index.advance(&doc).unwrap().complete_signed_prefix);
    finish(&mut index, &doc);
    let row = index.page(&doc, None, 1).unwrap().remove(0);
    assert!(matches!(row.signature, InspectionSignature::NotChecked(_)));
    assert_eq!(
        index
            .original_chunk(&doc, &row.op_hash, 64 * 1024, 64 * 1024)
            .unwrap()
            .len(),
        64 * 1024
    );
    assert!(matches!(
        index.inspect_original_change(&doc, &row.op_hash).unwrap(),
        LegacyChangeInspection::TooLarge
    ));
    assert_eq!(
        index.original_chunk(&doc, &row.op_hash, row.original_bytes + 1, 1),
        Err(SignedHistoryError::Request)
    );
}

#[test]
fn signed_history_does_not_close_a_partial_legacy_edit_or_promote_it_into_signed_inventory() {
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(81);
    let mut doc = document(&device);
    apply(
        &mut doc,
        original(&device, &mut writer(&device), "original"),
    );
    let mut index = doc.signed_history_index(1).unwrap();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = doc.edit(&device, &group, &mut rng, |graph| {
            graph.put(ROOT, "pending", "never signed")?;
            panic!("legacy caller interrupted before commit");
        });
    }));
    assert!(outcome.is_err());
    let pending = doc.doc.pending_ops();
    assert!(pending > 0);
    finish(&mut index, &doc);
    assert_eq!(
        doc.doc.pending_ops(),
        pending,
        "read closed a live transaction"
    );
    assert_eq!(index.page(&doc, None, 32).unwrap().len(), 1);
    // This is explicitly only signed inventory: there is no graph completeness or authority flag.
}

#[test]
fn signed_history_quota_and_page_refusals_do_not_change_retained_data() {
    let device = MlsDevice::generate().unwrap();
    let mut doc = document(&device);
    apply(
        &mut doc,
        original(&device, &mut writer(&device), "original"),
    );
    assert!(matches!(
        doc.signed_history_index(0),
        Err(SignedHistoryError::Capacity)
    ));
    let mut index = doc.signed_history_index(1).unwrap();
    finish(&mut index, &doc);
    assert_eq!(index.page(&doc, None, 0), Err(SignedHistoryError::Request));
    assert_eq!(index.page(&doc, None, 33), Err(SignedHistoryError::Request));
    assert_eq!(doc.op_count(), 1);
}
