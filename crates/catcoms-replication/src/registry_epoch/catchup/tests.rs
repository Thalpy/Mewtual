use super::*;
use crate::registry::{PointerKey, RegistryOp};
use crate::{Admission, InheritedCheckpoint, Receipt};
use catcoms_rt::ManualClock;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn registry_page_mac_context_has_a_golden_vector_and_binds_every_scope_field() {
    let provider = RegistryPageProvider {
        key: Zeroizing::new([0x11; 32]),
        provider: DeviceId::from_bytes([0x22; 32]),
        clock: Arc::new(ManualClock::new(0)),
        now_ms: 0,
    };
    let logical =
        LogicalDocument::new(b"server".to_vec(), DocType::DocRegistry, b"bucket".to_vec()).unwrap();
    let mut request = RegistryPageRequest {
        requester: DeviceId::from_bytes([0x33; 32]),
        doc_id: 0x44,
        heads: &[[0x55; 32]],
        seed: Some([0x66; 32]),
        cursor: None,
    };
    let payload = [0x77; PAYLOAD_BYTES];
    let tag = |logical: &LogicalDocument, request: &RegistryPageRequest<'_>| {
        let mut mac = provider.mac(logical, request).unwrap();
        mac.update(&payload);
        mac.finalize().into_bytes().to_vec()
    };
    let expected = tag(&logical, &request);
    // Independently reproduced with .NET HMACSHA256 and explicit big-endian length framing.
    assert_eq!(
        expected
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "cba60096a598d1a337df491e39548f1f18f1aaf565bde729eb9aaf8f9165a506"
    );
    let mut changed = logical.clone();
    changed.server_id.push(0);
    assert_ne!(tag(&changed, &request), expected);
    let mut changed = logical.clone();
    changed.logical_key.push(0);
    assert_ne!(tag(&changed, &request), expected);
    request.doc_id += 1;
    assert_ne!(tag(&logical, &request), expected);
    request.doc_id -= 1;
    request.requester = DeviceId::from_bytes([0x34; 32]);
    assert_ne!(tag(&logical, &request), expected);
    request.requester = DeviceId::from_bytes([0x33; 32]);
    request.seed = None;
    assert_ne!(tag(&logical, &request), expected);
    request.seed = Some([0x66; 32]);
    request.heads = &[];
    assert_ne!(tag(&logical, &request), expected);
    assert!(!format!("{request:?}").contains("bucket"));
}

struct Fixture {
    owner: MlsDevice,
    peer: MlsDevice,
    group: ServerGroup,
    peer_group: ServerGroup,
    source: RegistryEpoch,
    key: PointerKey,
    clock: Arc<ManualClock>,
    provider: RegistryPageProvider,
    rng: ChaCha20Rng,
}
impl Fixture {
    fn new() -> Self {
        let owner = MlsDevice::generate().unwrap();
        let peer = MlsDevice::generate().unwrap();
        let mut group = ServerGroup::create(&owner).unwrap();
        let added = group
            .add_member(&owner, peer.key_package().unwrap())
            .unwrap();
        let peer_group = ServerGroup::join(&peer, &added.welcome).unwrap();
        let key = PointerKey::new(DocType::StudioObject, b"private-page-cat".to_vec()).unwrap();
        let source = RegistryEpoch::new(&group, key.bucket(), owner.device_id()).unwrap();
        let clock = Arc::new(ManualClock::new(1000));
        let mut rng = ChaCha20Rng::from_seed([91; 32]);
        let provider = RegistryPageProvider::new(owner.device_id(), clock.clone(), &mut rng);
        Self {
            owner,
            peer,
            group,
            peer_group,
            source,
            key,
            clock,
            provider,
            rng,
        }
    }
    fn edit(&mut self, n: u8) {
        let domain = RegistryOp::Put {
            key: self.key.clone(),
            epoch: n as u64,
        }
        .domain_op(&self.group.group_id(), [n; 16])
        .unwrap();
        self.source
            .edit(&self.owner, &self.group, &mut self.rng, &domain)
            .unwrap();
    }
    fn outcome(
        &mut self,
        heads: &[[u8; 32]],
        cursor: Option<&[u8]>,
    ) -> Result<RegistryPageOutcome, ReplError> {
        self.provider.page(
            &self.source,
            &self.group,
            &self.owner,
            RegistryPageRequest {
                requester: self.peer.device_id(),
                doc_id: self.source.doc_id(),
                heads,
                seed: self.source.doc.checkpoint_origin().map(|s| s.seed_hash()),
                cursor,
            },
            &mut self.rng,
        )
    }
    fn page(&mut self, heads: &[[u8; 32]], cursor: Option<&[u8]>) -> RegistryOpPage {
        let RegistryPageOutcome::Page(page) = self.outcome(heads, cursor).unwrap() else {
            panic!("expected page");
        };
        assert!(page.operations.len() <= MAX_REGISTRY_PAGE_OPS);
        assert!(
            page.operations
                .iter()
                .map(|op| 4 + op.encode().len())
                .sum::<usize>()
                <= MAX_REGISTRY_PAGE_BYTES
        );
        assert!(page.next.is_none() || !page.operations.is_empty());
        page
    }
}

#[test]
fn registry_page_advances_fixed_prefix_across_append_reload_and_repeated_requests() {
    let mut f = Fixture::new();
    for n in 0..70 {
        f.edit(n);
    }
    let before = f.source.snapshot().unwrap();
    let mut target = RegistryEpoch::new(&f.peer_group, f.key.bucket(), f.peer.device_id()).unwrap();
    let first = f.page(&[], None);
    assert_eq!(first.operations.len(), 32);
    let cursor = first.next.unwrap();
    for op in first.operations {
        assert_eq!(
            target.ingest(&op, &f.peer_group, &f.peer).unwrap(),
            Admission::Accepted
        );
    }
    f.edit(70); // Does not invalidate or extend the in-flight prefix.
    let snapshot = f.source.snapshot().unwrap();
    f.source =
        RegistryEpoch::restore(&snapshot, &f.group, f.key.bucket(), f.owner.device_id()).unwrap();
    let second = f.page(&[], Some(cursor.as_bytes()));
    let retry = f.page(&[], Some(cursor.as_bytes()));
    let key = f
        .group
        .channel_secret(&f.owner, DocType::DocRegistry, f.source.doc_id())
        .unwrap();
    for (a, b) in second.operations.iter().zip(&retry.operations) {
        assert_eq!(a.open(&key).unwrap(), b.open(&key).unwrap());
        assert_ne!(a.blob.nonce, b.blob.nonce);
    }
    for op in &second.operations {
        target.ingest(op, &f.peer_group, &f.peer).unwrap();
    }
    let third = f.page(&[], Some(second.next.unwrap().as_bytes()));
    assert_eq!(third.operations.len(), 6);
    assert!(third.next.is_none());
    for op in third.operations {
        target.ingest(&op, &f.peer_group, &f.peer).unwrap();
    }
    assert_eq!(target.op_count(), 70);
    assert_eq!(f.source.op_count(), 71);
    assert_eq!(
        f.source.snapshot().unwrap(),
        snapshot,
        "paging never mutates accepted state"
    );
    let heads = target.doc.heads();
    let gap = f.page(&heads, None);
    assert_eq!(gap.operations.len(), 1);
    target
        .ingest(&gap.operations[0], &f.peer_group, &f.peer)
        .unwrap();
    assert_eq!(target.projection().unwrap(), f.source.projection().unwrap());
    assert_ne!(before, snapshot);
}

#[test]
fn registry_page_cursor_binds_request_scope_key_and_fixed_expiry() {
    let mut f = Fixture::new();
    for n in 0..65 {
        f.edit(n);
    }
    let cursor = f.page(&[], None).next.unwrap();
    for i in 0..REGISTRY_CURSOR_BYTES {
        let mut corrupt = cursor.as_bytes().to_vec();
        corrupt[i] ^= 1;
        assert!(matches!(
            f.outcome(&[], Some(&corrupt)),
            Err(ReplError::EpochAuthority)
        ));
    }
    assert!(f.outcome(&[], Some(&cursor.as_bytes()[1..])).is_err());
    let head = Change::from_bytes(f.source.doc.signed_log()[0].delta.clone())
        .unwrap()
        .hash()
        .0;
    assert!(matches!(
        f.outcome(&[head], Some(cursor.as_bytes())),
        Err(ReplError::EpochAuthority)
    ));
    let request = RegistryPageRequest {
        requester: f.owner.device_id(),
        doc_id: f.source.doc_id(),
        heads: &[],
        seed: None,
        cursor: Some(cursor.as_bytes()),
    };
    assert!(matches!(
        f.provider
            .page(&f.source, &f.group, &f.owner, request, &mut f.rng),
        Err(ReplError::EpochAuthority)
    ));
    let other_scope =
        registry_document(&f.group.group_id(), f.key.bucket().wrapping_add(1)).unwrap();
    let request = RegistryPageRequest {
        requester: f.peer.device_id(),
        doc_id: f.source.doc_id(),
        heads: &[],
        seed: None,
        cursor: Some(cursor.as_bytes()),
    };
    let mut mac = f.provider.mac(&other_scope, &request).unwrap();
    mac.update(&cursor.as_bytes()[..PAYLOAD_BYTES]);
    assert!(mac
        .verify_slice(&cursor.as_bytes()[PAYLOAD_BYTES..])
        .is_err());
    f.clock.set_wall_ms(u64::MAX);
    let second = f.page(&[], Some(cursor.as_bytes())); // Wall time grants no expiry/extension.
    f.clock.set_wall_ms(0);
    f.clock.advance_ms(CURSOR_TTL_MS - 1);
    assert!(matches!(
        f.outcome(&[], Some(cursor.as_bytes())).unwrap(),
        RegistryPageOutcome::Page(_)
    ));
    f.clock.advance_ms(1);
    assert!(matches!(
        f.outcome(&[], Some(second.next.unwrap().as_bytes()))
            .unwrap(),
        RegistryPageOutcome::Restart
    ));
    f.provider = RegistryPageProvider::new(f.owner.device_id(), f.clock.clone(), &mut f.rng);
    assert!(matches!(
        f.outcome(&[], Some(cursor.as_bytes())),
        Err(ReplError::EpochAuthority)
    ));
    assert_eq!(format!("{cursor:?}"), "RegistryPageCursor { .. }");
    assert_eq!(format!("{:?}", f.provider), "RegistryPageProvider { .. }");
}

#[test]
fn registry_page_changed_prefix_unknown_heads_and_noncanonical_requests_refuse() {
    let mut f = Fixture::new();
    for n in 0..35 {
        f.edit(n);
    }
    let cursor = f.page(&[], None).next.unwrap();
    assert!(matches!(
        f.outcome(&[[0xee; 32]], None).unwrap(),
        RegistryPageOutcome::Restart
    ));
    assert!(f.outcome(&[[0; 32]; 65], None).is_err());
    assert!(f.outcome(&[[1; 32], [1; 32]], None).is_err());
    assert!(f.outcome(&[[2; 32], [1; 32]], None).is_err());
    // Same concrete epoch, same length, different accepted history must not reuse its cursor.
    f.source = RegistryEpoch::new(&f.group, f.key.bucket(), f.owner.device_id()).unwrap();
    for n in 100..135 {
        f.edit(n);
    }
    assert!(matches!(
        f.outcome(&[], Some(cursor.as_bytes())).unwrap(),
        RegistryPageOutcome::Restart
    ));
    f.clock.set_ms(u64::MAX);
    assert!(matches!(f.outcome(&[], None), Err(ReplError::EpochBound)));
}

#[test]
fn registry_page_rotated_source_requires_seed_and_never_transfers_it_implicitly() {
    let mut f = Fixture::new();
    f.edit(1);
    let seed = f.source.projection().unwrap().checkpoint([7; 32]).unwrap();
    let receipt = Receipt::sign(
        f.source.logical.clone(),
        0,
        [7; 32],
        seed.change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        &f.owner,
    )
    .unwrap();
    f.source = RegistryEpoch::from_checkpoint(
        &f.group,
        f.key.bucket(),
        f.owner.device_id(),
        receipt.clone(),
        0,
        seed.bytes(),
    )
    .unwrap();
    f.edit(2);
    for claimed in [None, Some([0; 32])] {
        let request = RegistryPageRequest {
            requester: f.peer.device_id(),
            doc_id: f.source.doc_id(),
            heads: &[],
            seed: claimed,
            cursor: None,
        };
        assert!(matches!(
            f.provider
                .page(&f.source, &f.group, &f.owner, request, &mut f.rng)
                .unwrap(),
            RegistryPageOutcome::CheckpointRequired
        ));
    }
    let page = f.page(&[], None);
    assert_eq!(page.operations.len(), 1);
    let mut target = RegistryEpoch::from_checkpoint(
        &f.peer_group,
        f.key.bucket(),
        f.peer.device_id(),
        receipt,
        0,
        seed.bytes(),
    )
    .unwrap();
    assert_eq!(
        target
            .ingest(&page.operations[0], &f.peer_group, &f.peer)
            .unwrap(),
        Admission::Accepted
    );
    assert_eq!(target.projection().unwrap(), f.source.projection().unwrap());
}

#[test]
fn registry_page_removed_author_blocks_missing_dependency_without_weakening_live_ingest() {
    let mut f = Fixture::new();
    let mut peer_source =
        RegistryEpoch::new(&f.peer_group, f.key.bucket(), f.peer.device_id()).unwrap();
    let domain = RegistryOp::Put {
        key: f.key.clone(),
        epoch: 1,
    }
    .domain_op(&f.group.group_id(), [3; 16])
    .unwrap();
    let op = peer_source
        .edit(&f.peer, &f.peer_group, &mut f.rng, &domain)
        .unwrap();
    f.source.ingest(&op, &f.group, &f.owner).unwrap();
    let removed_head = f.source.doc.heads()[0];
    f.edit(2); // Current owner's change depends on the soon-removed member's change.
    f.group
        .remove_member(&f.owner, &f.peer.device_id())
        .unwrap();
    let saved = f.source.snapshot().unwrap();
    f.source =
        RegistryEpoch::restore(&saved, &f.group, f.key.bucket(), f.owner.device_id()).unwrap();
    let request = RegistryPageRequest {
        requester: f.owner.device_id(),
        doc_id: f.source.doc_id(),
        heads: &[],
        seed: None,
        cursor: None,
    };
    assert!(matches!(
        f.provider
            .page(&f.source, &f.group, &f.owner, request, &mut f.rng)
            .unwrap(),
        RegistryPageOutcome::HistoricalAuthorizationRequired
    ));
    assert!(
        matches!(f.outcome(&[], None), Err(ReplError::EpochAuthority)),
        "removed requester gets no history"
    );
    let request = RegistryPageRequest {
        requester: f.owner.device_id(),
        doc_id: f.source.doc_id(),
        heads: &[removed_head],
        seed: None,
        cursor: None,
    };
    let RegistryPageOutcome::Page(page) = f
        .provider
        .page(&f.source, &f.group, &f.owner, request, &mut f.rng)
        .unwrap()
    else {
        panic!("already held ancestor is not missing");
    };
    assert_eq!(page.operations.len(), 1);
    assert_eq!(page.operations[0].epoch, f.group.epoch());
}

#[test]
fn registry_page_more_than_64_independent_heads_still_finishes_from_empty_frontier() {
    let mut f = Fixture::new();
    for n in 0..65u8 {
        let author = MlsDevice::generate().unwrap();
        let added = f
            .group
            .add_member(&f.owner, author.key_package().unwrap())
            .unwrap();
        let g = ServerGroup::join(&author, &added.welcome).unwrap();
        let mut independent = RegistryEpoch::new(&g, f.key.bucket(), author.device_id()).unwrap();
        let domain = RegistryOp::Put {
            key: f.key.clone(),
            epoch: n as u64,
        }
        .domain_op(&g.group_id(), [n; 16])
        .unwrap();
        let op = independent.edit(&author, &g, &mut f.rng, &domain).unwrap();
        assert_eq!(
            f.source.ingest(&op, &f.group, &f.owner).unwrap(),
            Admission::Accepted
        );
    }
    assert_eq!(f.source.doc.heads().len(), 65);
    let first = f.page(&[], None);
    let second = f.page(&[], Some(first.next.unwrap().as_bytes()));
    let third = f.page(&[], Some(second.next.unwrap().as_bytes()));
    assert_eq!(
        first.operations.len() + second.operations.len() + third.operations.len(),
        65
    );
    assert!(third.next.is_none());
}

#[test]
fn registry_page_removal_after_delivery_does_not_block_remaining_current_authored_descendants() {
    let mut f = Fixture::new();
    let mut peer_source =
        RegistryEpoch::new(&f.peer_group, f.key.bucket(), f.peer.device_id()).unwrap();
    let mut target = RegistryEpoch::new(&f.group, f.key.bucket(), f.owner.device_id()).unwrap();
    for n in 0..32u8 {
        let domain = RegistryOp::Put {
            key: f.key.clone(),
            epoch: n as u64,
        }
        .domain_op(&f.group.group_id(), [n; 16])
        .unwrap();
        let sealed = peer_source
            .edit(&f.peer, &f.peer_group, &mut f.rng, &domain)
            .unwrap();
        assert_eq!(
            f.source.ingest(&sealed, &f.group, &f.owner).unwrap(),
            Admission::Accepted
        );
    }
    f.edit(32);
    let request = RegistryPageRequest {
        requester: f.owner.device_id(),
        doc_id: f.source.doc_id(),
        heads: &[],
        seed: None,
        cursor: None,
    };
    let RegistryPageOutcome::Page(first) = f
        .provider
        .page(&f.source, &f.group, &f.owner, request, &mut f.rng)
        .unwrap()
    else {
        panic!("first page");
    };
    assert_eq!(first.operations.len(), 32);
    for sealed in &first.operations {
        assert_eq!(
            target.ingest(sealed, &f.group, &f.owner).unwrap(),
            Admission::Accepted
        );
    }
    f.group
        .remove_member(&f.owner, &f.peer.device_id())
        .unwrap();
    // A reload preserves historical admission. Only the operation not yet held needs current
    // author authority; it is freshly resealed under this later MLS epoch.
    target = RegistryEpoch::restore(
        &target.snapshot().unwrap(),
        &f.group,
        f.key.bucket(),
        f.owner.device_id(),
    )
    .unwrap();
    let request = RegistryPageRequest {
        requester: f.owner.device_id(),
        doc_id: f.source.doc_id(),
        heads: &[],
        seed: None,
        cursor: first.next.as_ref().map(RegistryPageCursor::as_bytes),
    };
    let RegistryPageOutcome::Page(second) = f
        .provider
        .page(&f.source, &f.group, &f.owner, request, &mut f.rng)
        .unwrap()
    else {
        panic!("already delivered removed-author history must not block page two");
    };
    assert_eq!(second.operations.len(), 1);
    assert!(second.next.is_none());
    assert_eq!(second.operations[0].epoch, f.group.epoch());
    assert_eq!(
        target
            .ingest(&second.operations[0], &f.group, &f.owner)
            .unwrap(),
        Admission::Accepted
    );
    assert_eq!(target.projection().unwrap(), f.source.projection().unwrap());
}

#[test]
fn registry_page_padding_ceiling_counts_real_framed_bytes_and_always_makes_progress() {
    use automerge::transaction::{CommitOptions, Transactable};
    use automerge::ROOT;
    let mut f = Fixture::new();
    // Real accepted signed operations with large commit messages, not oversized fake structs.
    let mut writer = f.source.doc.doc().clone();
    for n in 0..3u8 {
        let domain = RegistryOp::Put {
            key: f.key.clone(),
            epoch: n as u64,
        }
        .domain_op(&f.group.group_id(), [n; 16])
        .unwrap();
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        for (key, value) in [("bucket", f.key.bucket() as u64), ("epoch", 0), ("v", 1)] {
            writer.put(ROOT, key, value).unwrap();
        }
        writer
            .put(ROOT, "key", hex(&f.source.logical.logical_key))
            .unwrap();
        writer.put(ROOT, "kind", "registry").unwrap();
        writer
            .put(
                ROOT,
                format!("p/0010/{}", hex(f.key.logical_key())),
                n as u64,
            )
            .unwrap();
        writer
            .put(
                ROOT,
                format!("_p1/op/{}", hex(&domain.id(&f.owner.device_id()))),
                1u64,
            )
            .unwrap();
        writer.commit_with(CommitOptions::default().with_message("x".repeat(240_000)));
        let op = SignedOp::sign_domain(
            &f.owner,
            DocType::DocRegistry,
            f.source.doc_id(),
            writer.get_last_local_change().unwrap().raw_bytes().to_vec(),
            &domain,
        )
        .unwrap();
        let sealed = SealedOp::seal(&op, &f.group, &f.owner, &mut f.rng).unwrap();
        assert_eq!(
            f.source.ingest(&sealed, &f.group, &f.owner).unwrap(),
            Admission::Accepted
        );
    }
    let mut cursor = None;
    for i in 0..3 {
        let page = f.page(&[], cursor.as_ref().map(RegistryPageCursor::as_bytes));
        assert_eq!(
            page.operations.len(),
            1,
            "two padded maximum-bucket ops exceed the framed page cap"
        );
        assert_eq!(page.operations[0].encode().len() + 4, 256 * 1024 + 82);
        assert_eq!(page.next.is_none(), i == 2);
        cursor = page.next;
    }
}
