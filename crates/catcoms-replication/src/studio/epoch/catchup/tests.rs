use super::*;
use crate::InheritedCheckpoint;
use catcoms_rt::ManualClock;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

struct Fixture {
    owner: MlsDevice,
    peer: MlsDevice,
    group: ServerGroup,
    peer_group: ServerGroup,
    source: StudioEpoch,
    provider: StudioPageProvider,
    clock: Arc<ManualClock>,
    rng: ChaCha20Rng,
}
impl Fixture {
    fn new(art: bool) -> Self {
        let owner = MlsDevice::generate().unwrap();
        let peer = MlsDevice::generate().unwrap();
        let mut group = ServerGroup::create(&owner).unwrap();
        let add = group
            .add_member(&owner, peer.key_package().unwrap())
            .unwrap();
        let peer_group = ServerGroup::join(&peer, &add.welcome).unwrap();
        let target = if art {
            StudioTarget::Flipnote {
                channel: [7; 16],
                object: [9; 16],
            }
        } else {
            StudioTarget::Index { channel: [7; 16] }
        };
        let source = StudioEpoch::new(&group, target, owner.device_id()).unwrap();
        let clock = Arc::new(ManualClock::new(1000));
        let mut rng = ChaCha20Rng::from_seed([44; 32]);
        let provider = StudioPageProvider::new(owner.device_id(), clock.clone(), &mut rng);
        Self {
            owner,
            peer,
            group,
            peer_group,
            source,
            provider,
            clock,
            rng,
        }
    }
    fn edit(&mut self, n: u8) {
        let body = match self.source.target {
            StudioTarget::Flipnote { .. } => {
                FlipnoteOp::SetHeader(FlipnoteHeader::Title(format!("private moon {n}")))
                    .encode()
                    .unwrap()
            }
            _ if n == 0 => IndexOp::PutObject {
                object: [1; 16],
                kind: StudioKind::Flipnote,
                title: "private moon".into(),
                created_by: self.owner.device_id(),
                ts: 100,
                expiry: StudioExpiry::Never,
            }
            .encode()
            .unwrap(),
            _ => IndexOp::SetTitle {
                object: [1; 16],
                title: format!("private moon {n}"),
            }
            .encode()
            .unwrap(),
        };
        let op = DomainOp {
            nonce: [n; 16],
            doc_type: self.source.logical.doc_type,
            logical_key: self.source.logical.logical_key.clone(),
            body,
        };
        self.source
            .edit_or_reseal(&self.owner, &self.group, &mut self.rng, &op, 100)
            .unwrap();
    }
    fn outcome(
        &mut self,
        heads: &[[u8; 32]],
        seed: Option<[u8; 32]>,
        cursor: Option<&[u8]>,
    ) -> Result<StudioPageOutcome, ReplError> {
        self.provider.page(
            &self.source,
            &self.group,
            &self.owner,
            StudioPageRequest {
                requester: self.peer.device_id(),
                doc_id: self.source.doc_id(),
                heads,
                seed,
                cursor,
            },
            &mut self.rng,
        )
    }
    fn page(
        &mut self,
        heads: &[[u8; 32]],
        seed: Option<[u8; 32]>,
        cursor: Option<&[u8]>,
    ) -> StudioOpPage {
        let StudioPageOutcome::Page(page) = self.outcome(heads, seed, cursor).unwrap() else {
            panic!("expected page")
        };
        assert!(page.operations.len() <= MAX_STUDIO_PAGE_OPS);
        assert!(
            page.operations
                .iter()
                .map(|op| 4 + op.encode().len())
                .sum::<usize>()
                <= MAX_STUDIO_PAGE_BYTES
        );
        assert!(page.next.is_none() || !page.operations.is_empty());
        page
    }
}

#[test]
fn studio_pages_resume_fixed_prefix_after_append_reload_and_duplicate_request() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        for n in 0..70 {
            f.edit(n);
        }
        let mut receiver =
            StudioEpoch::new(&f.peer_group, f.source.target, f.peer.device_id()).unwrap();
        let first = f.page(&[], None, None);
        assert_eq!(first.operations.len(), 32);
        for op in &first.operations {
            assert_eq!(
                receiver.ingest(op, &f.peer_group, &f.peer).unwrap(),
                Admission::Accepted
            );
        }
        f.edit(70);
        let snapshot = f.source.snapshot().unwrap();
        f.source = StudioEpoch::restore(&snapshot, &f.group, f.source.target, f.owner.device_id())
            .unwrap();
        let second = f.page(&[], None, Some(first.next.as_ref().unwrap().as_bytes()));
        let again = f.page(&[], None, Some(first.next.as_ref().unwrap().as_bytes()));
        let key = f
            .group
            .channel_secret(&f.owner, f.source.logical.doc_type, f.source.doc_id())
            .unwrap();
        for (op, repeated) in second.operations.iter().zip(&again.operations) {
            assert_eq!(op.open(&key).unwrap(), repeated.open(&key).unwrap());
            assert_ne!(op.blob.nonce, repeated.blob.nonce);
            receiver.ingest(op, &f.peer_group, &f.peer).unwrap();
        }
        let third = f.page(&[], None, Some(second.next.unwrap().as_bytes()));
        assert_eq!(third.operations.len(), 6);
        assert!(third.next.is_none());
        for op in &third.operations {
            receiver.ingest(op, &f.peer_group, &f.peer).unwrap();
        }
        assert_eq!(receiver.op_count(), 70);
        assert_eq!(
            f.source.snapshot().unwrap(),
            snapshot,
            "paging never mutates the source"
        );
        let frontier = receiver.catchup_frontier();
        let last = f.page(&frontier.heads, frontier.seed, None);
        assert_eq!(last.operations.len(), 1);
        receiver
            .ingest(&last.operations[0], &f.peer_group, &f.peer)
            .unwrap();
        assert_eq!(
            receiver.projection().unwrap(),
            f.source.projection().unwrap()
        );
    }
}

#[test]
fn studio_page_cursor_rejects_scope_tampering_expiry_and_provider_restart() {
    let mut f = Fixture::new(true);
    for n in 0..33 {
        f.edit(n);
    }
    let first = f.page(&[], None, None);
    let cursor = first.next.unwrap();
    for target in [
        StudioTarget::Flipnote {
            channel: [8; 16],
            object: [9; 16],
        },
        StudioTarget::Flipnote {
            channel: [7; 16],
            object: [8; 16],
        },
        StudioTarget::Index { channel: [7; 16] },
    ] {
        assert!(f
            .provider
            .preflight_request(
                &f.group,
                &f.owner,
                target,
                &StudioPageRequest {
                    requester: f.peer.device_id(),
                    doc_id: f.source.doc_id(),
                    heads: &[],
                    seed: None,
                    cursor: Some(cursor.as_bytes()),
                }
            )
            .is_err());
    }
    assert!(f
        .outcome(&[[1; 32]], None, Some(cursor.as_bytes()))
        .is_err());
    assert!(f
        .outcome(&[], Some([1; 32]), Some(cursor.as_bytes()))
        .is_err());
    let mut changed = cursor.as_bytes().to_vec();
    changed[5] ^= 1;
    assert!(f.outcome(&[], None, Some(&changed)).is_err());
    f.clock.advance_ms(600_000);
    assert!(matches!(
        f.outcome(&[], None, Some(cursor.as_bytes())).unwrap(),
        StudioPageOutcome::Restart
    ));
    f.provider = StudioPageProvider::new(f.owner.device_id(), f.clock.clone(), &mut f.rng);
    assert!(f.outcome(&[], None, Some(cursor.as_bytes())).is_err());
}

#[test]
fn studio_pages_hold_unknown_heads_seed_mismatch_fault_and_removed_requester() {
    let mut f = Fixture::new(true);
    f.edit(0);
    assert!(matches!(
        f.outcome(&[[1; 32]], None, None).unwrap(),
        StudioPageOutcome::Restart
    ));
    assert!(f.outcome(&[[1; 32]; 65], None, None).is_err());
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
    f.source = StudioEpoch::from_checkpoint(
        &f.group,
        f.source.target,
        f.owner.device_id(),
        receipt,
        0,
        seed.bytes(),
    )
    .unwrap();
    f.edit(1);
    assert!(matches!(
        f.outcome(&[], None, None).unwrap(),
        StudioPageOutcome::CheckpointRequired
    ));
    let page = f.page(&[], Some(seed.change_hash()), None);
    assert_eq!(
        page.operations.len(),
        1,
        "never send the raw seed as an operation"
    );
    let mut receiver =
        StudioEpoch::new(&f.peer_group, f.source.target, f.peer.device_id()).unwrap();
    assert!(
        receiver
            .ingest(&page.operations[0], &f.peer_group, &f.peer)
            .is_err(),
        "a page cannot install its checkpoint"
    );
    for close in [8, 9] {
        let selected = f
            .source
            .projection()
            .unwrap()
            .checkpoint([close; 32])
            .unwrap();
        let receipt = Receipt::sign(
            f.source.logical.clone(),
            1,
            [close; 32],
            selected.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            &f.owner,
        )
        .unwrap();
        f.source.seal(receipt, &f.group, 0).unwrap();
    }
    assert_eq!(f.source.phase(), EpochPhase::Fault);
    assert!(matches!(
        f.outcome(&[], Some(seed.change_hash()), None),
        Err(ReplError::ReceiptConflict)
    ));
    f.group
        .remove_member(&f.owner, &f.peer.device_id())
        .unwrap();
    assert!(matches!(
        f.outcome(&[], Some(seed.change_hash()), None),
        Err(ReplError::EpochAuthority)
    ));
}

#[test]
fn studio_pages_removed_author_in_later_missing_range_cannot_be_laundered() {
    let mut f = Fixture::new(true);
    let former = MlsDevice::generate().unwrap();
    let added = f
        .group
        .add_member(&f.owner, former.key_package().unwrap())
        .unwrap();
    let former_group = ServerGroup::join(&former, &added.welcome).unwrap();
    for n in 0..32 {
        f.edit(n);
    }
    let mut branch = StudioEpoch::restore(
        &f.source.snapshot().unwrap(),
        &former_group,
        f.source.target,
        former.device_id(),
    )
    .unwrap();
    let domain = DomainOp {
        nonce: [82; 16],
        doc_type: branch.logical.doc_type,
        logical_key: branch.logical.logical_key.clone(),
        body: FlipnoteOp::SetHeader(FlipnoteHeader::Title("removed author's edit".into()))
            .encode()
            .unwrap(),
    };
    let op = branch
        .edit_or_reseal(&former, &former_group, &mut f.rng, &domain, 100)
        .unwrap();
    f.source.ingest(&op, &f.group, &f.owner).unwrap();
    f.edit(34);
    let first = f.page(&[], None, None);
    assert_eq!(first.operations.len(), 32);
    f.group
        .remove_member(&f.owner, &former.device_id())
        .unwrap();
    assert!(matches!(
        f.outcome(&[], None, Some(first.next.unwrap().as_bytes()))
            .unwrap(),
        StudioPageOutcome::HistoricalAuthorizationRequired
    ));
    // A claimed already-held ancestor is different: current-authored descendants may be
    // served, but these heads are not a possession proof or checkpoint-installation authority.
    let held = branch.catchup_frontier();
    let page = f.page(&held.heads, None, None);
    assert_eq!(page.operations.len(), 1);
    assert_eq!(page.operations[0].epoch, f.group.epoch());
}

#[test]
fn studio_pages_wide_frontier_finishes_without_advancing_initial_heads() {
    let mut f = Fixture::new(true);
    for n in 0..65 {
        let author = MlsDevice::generate().unwrap();
        let added = f
            .group
            .add_member(&f.owner, author.key_package().unwrap())
            .unwrap();
        let group = ServerGroup::join(&author, &added.welcome).unwrap();
        let mut branch = StudioEpoch::new(&group, f.source.target, author.device_id()).unwrap();
        let op = DomainOp {
            nonce: [n; 16],
            doc_type: branch.logical.doc_type,
            logical_key: branch.logical.logical_key.clone(),
            body: FlipnoteOp::SetHeader(FlipnoteHeader::Title(format!("branch {n}")))
                .encode()
                .unwrap(),
        };
        let sealed = branch
            .edit_or_reseal(&author, &group, &mut f.rng, &op, 100)
            .unwrap();
        assert_eq!(
            f.source.ingest(&sealed, &f.group, &f.owner).unwrap(),
            Admission::Accepted
        );
    }
    assert_eq!(f.source.doc.heads().len(), 65);
    let before = f.source.snapshot().unwrap();
    assert!(
        matches!(
            f.source.new_owner_decision(&f.group, &f.owner, 0, None),
            Err(ReplError::EpochBound)
        ),
        "owner must refuse, never truncate, a real wide head set"
    );
    assert_eq!(f.source.snapshot().unwrap(), before);
    let frontier = f.source.catchup_frontier();
    assert!(frontier.heads.is_empty());
    assert!(frontier.seed.is_none());
    let first = f.page(&frontier.heads, frontier.seed, None);
    let second = f.page(
        &frontier.heads,
        frontier.seed,
        Some(first.next.unwrap().as_bytes()),
    );
    let third = f.page(
        &frontier.heads,
        frontier.seed,
        Some(second.next.unwrap().as_bytes()),
    );
    assert_eq!(
        first.operations.len() + second.operations.len() + third.operations.len(),
        65
    );
    assert!(third.next.is_none());
}
