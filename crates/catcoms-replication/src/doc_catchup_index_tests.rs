use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn recent_linear_heads_survive_cache_eviction_and_cold_restore_before_first_request() {
    let author = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&author).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(930);
    let mut provider = EncryptedDoc::new(DocType::Channel, 930, &author.device_id());
    provider.catchup_index = AncestryIndex::with_capacity(2);
    let mut requester = EncryptedDoc::new(DocType::Channel, 930, &author.device_id());
    for count in 0..MAX_CATCHUP_PAGE_CLOSURE_STEPS + 17 {
        let op = provider
            .edit(&author, &group, &mut rng, |doc| {
                doc.put(ROOT, "count", count as u64)
            })
            .unwrap();
        requester.ingest(&op, &group, &author).unwrap();
    }
    let heads = requester.heads();
    provider
        .edit(&author, &group, &mut rng, |doc| {
            doc.put(ROOT, "missing", true)
        })
        .unwrap();
    let (page, next) = provider
        .export_catchup_page(&heads, 0, usize::MAX, &group, &author, &mut rng)
        .unwrap();
    assert_eq!(
        page.len(),
        1,
        "two cached certificates still describe all previously accepted ancestors"
    );
    assert_eq!(next, None);
    let snapshot = provider.snapshot().unwrap();
    let mut restored = EncryptedDoc::restore(&snapshot).unwrap();
    let (page, next) = restored
        .export_catchup_page(&heads, 0, usize::MAX, &group, &author, &mut rng)
        .unwrap();
    assert_eq!((page.len(), next), (1, None));
    assert_eq!(requester.import_catchup(&page, &group, &author).unwrap(), 1);
    assert_eq!(requester.heads(), restored.heads());
    // Derived state is absent from the snapshot and cannot affect saved bytes or the cache key.
    provider.catchup_index = AncestryIndex::with_capacity(0);
    assert_eq!(provider.snapshot().unwrap(), snapshot);
    let (fallback, next) = provider
        .export_catchup_page(&heads, 0, usize::MAX, &group, &author, &mut rng)
        .unwrap();
    assert!(fallback.len() <= MAX_CATCHUP_PAGE_SCANNED_OPS);
    assert_eq!(next, Some(MAX_CATCHUP_PAGE_SCANNED_OPS));
}

#[test]
fn incomparable_heads_do_not_certify_siblings_in_either_providers_log_order() {
    let author = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&author).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(931);
    let mut base = EncryptedDoc::new(DocType::Channel, 931, &author.device_id());
    let seed = base
        .edit(&author, &group, &mut rng, |doc| doc.put(ROOT, "seed", true))
        .unwrap();
    let mut left = EncryptedDoc::new(
        DocType::Channel,
        931,
        &DeviceId::from_public_key_bytes(&[1; 32]),
    );
    let mut right = EncryptedDoc::new(
        DocType::Channel,
        931,
        &DeviceId::from_public_key_bytes(&[2; 32]),
    );
    left.ingest(&seed, &group, &author).unwrap();
    right.ingest(&seed, &group, &author).unwrap();
    let mut left_ops = Vec::new();
    let mut right_ops = Vec::new();
    for count in 0..48 {
        left_ops.push(
            left.edit(&author, &group, &mut rng, |doc| {
                doc.put(ROOT, "left", count as u64)
            })
            .unwrap(),
        );
        right_ops.push(
            right
                .edit(&author, &group, &mut rng, |doc| {
                    doc.put(ROOT, "right", count as u64)
                })
                .unwrap(),
        );
    }
    for reverse_order in [false, true] {
        let mut provider = EncryptedDoc::new(DocType::Channel, 931, &author.device_id());
        provider.ingest(&seed, &group, &author).unwrap();
        for (left_op, right_op) in left_ops.iter().zip(&right_ops) {
            for operation in if reverse_order {
                [right_op, left_op]
            } else {
                [left_op, right_op]
            } {
                provider.ingest(operation, &group, &author).unwrap();
            }
        }
        let (page, next) = provider
            .export_catchup_page(&left.heads(), 0, usize::MAX, &group, &author, &mut rng)
            .unwrap();
        assert_eq!(
            (page.len(), next),
            (48, None),
            "a left-branch head never covers a concurrent right sibling"
        );
        let mut requester = EncryptedDoc::restore(&left.snapshot().unwrap()).unwrap();
        for operation in &right_ops[..47] {
            requester.ingest(operation, &group, &author).unwrap();
        }
        let mut heads = requester.heads();
        assert_eq!(heads.len(), 2);
        heads.push([0xff; 32]);
        let (page, next) = provider
            .export_catchup_page(&heads, 0, usize::MAX, &group, &author, &mut rng)
            .unwrap();
        assert_eq!(
            (page.len(), next),
            (1, None),
            "bounded walk combines surviving fragmented certificates"
        );
        assert_eq!(requester.import_catchup(&page, &group, &author).unwrap(), 1);
        assert_eq!(requester.heads(), provider.heads());
    }
}

#[test]
fn appends_during_paging_remain_outside_previous_ancestry_certificates() {
    let author = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&author).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(932);
    let mut provider = EncryptedDoc::new(DocType::Channel, 932, &author.device_id());
    let mut requester = EncryptedDoc::new(DocType::Channel, 932, &author.device_id());
    for count in 0..MAX_CATCHUP_PAGE_SCANNED_OPS + 5 {
        let op = provider
            .edit(&author, &group, &mut rng, |doc| {
                doc.put(ROOT, "count", count as u64)
            })
            .unwrap();
        if count < MAX_CATCHUP_PAGE_SCANNED_OPS {
            requester.ingest(&op, &group, &author).unwrap();
        }
    }
    let mut position = 0;
    let mut transferred = 0;
    let mut applied = 0;
    loop {
        let (page, next) = provider
            .export_catchup_page(&requester.heads(), position, 1, &group, &author, &mut rng)
            .unwrap();
        transferred += page.len();
        applied += requester.import_catchup(&page, &group, &author).unwrap();
        if transferred == 1 {
            provider
                .edit(&author, &group, &mut rng, |doc| {
                    doc.put(ROOT, "append_during_walk", true)
                })
                .unwrap();
        }
        match next {
            Some(next) => {
                assert!(next > position);
                position = next;
            }
            None => break,
        }
    }
    assert_eq!((transferred, applied), (6, 6));
    assert_eq!(requester.heads(), provider.heads());
}

#[test]
fn buffered_children_and_foreign_document_heads_never_grant_a_range_certificate() {
    let author = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&author).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(933);
    let mut source = EncryptedDoc::new(DocType::Channel, 933, &author.device_id());
    let parent = source
        .edit(&author, &group, &mut rng, |doc| {
            doc.put(ROOT, "parent", true)
        })
        .unwrap();
    let child = source
        .edit(&author, &group, &mut rng, |doc| {
            doc.put(ROOT, "child", true)
        })
        .unwrap();
    let child_head = source.heads();
    let mut provider = EncryptedDoc::new(DocType::Channel, 933, &author.device_id());
    provider.ingest(&child, &group, &author).unwrap();
    assert!(provider
        .catchup_index
        .get(&ChangeHash(child_head[0]))
        .is_none());
    let (known, ranges) = provider.held_closure_bounded(&child_head);
    assert!(known.is_empty() && ranges.is_empty());
    let (page, next) = provider
        .export_catchup_page(&child_head, 0, usize::MAX, &group, &author, &mut rng)
        .unwrap();
    assert_eq!((page.len(), next), (1, None));
    provider.ingest(&parent, &group, &author).unwrap();
    let (page, next) = provider
        .export_catchup_page(&child_head, 0, usize::MAX, &group, &author, &mut rng)
        .unwrap();
    assert_eq!(
        (page.len(), next),
        (0, None),
        "once canonical, the child's local dependencies can be walked"
    );

    let mut foreign = EncryptedDoc::new(DocType::Channel, 934, &author.device_id());
    foreign
        .edit(&author, &group, &mut rng, |doc| {
            doc.put(ROOT, "foreign", true)
        })
        .unwrap();
    let mut restored = EncryptedDoc::restore(&foreign.snapshot().unwrap()).unwrap();
    let (page, next) = restored
        .export_catchup_page(&child_head, 0, usize::MAX, &group, &author, &mut rng)
        .unwrap();
    assert_eq!(
        (page.len(), next),
        (1, None),
        "new document state never inherits another document's ranges"
    );
}
