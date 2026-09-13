use super::*;
use crate::studio::{FlipnoteHeader, FlipnoteOp, IndexOp, StudioEpoch, StudioExpiry, StudioKind};
use crate::{DomainOp, SealedOp, SignedOp};
use catcoms_mls::ServerGroup;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

struct Fixture {
    owner: MlsDevice,
    peer: MlsDevice,
    outsider: MlsDevice,
    group: ServerGroup,
    peer_group: ServerGroup,
    target: StudioTarget,
    source: StudioEpoch,
    receipt: Receipt,
    seed: CheckpointSeed,
    rng: ChaCha20Rng,
}
impl Fixture {
    fn new(art: bool) -> Self {
        let owner = MlsDevice::generate().unwrap();
        let peer = MlsDevice::generate().unwrap();
        let outsider = MlsDevice::generate().unwrap();
        let mut group = ServerGroup::create(&owner).unwrap();
        let add = group
            .add_member(&owner, peer.key_package().unwrap())
            .unwrap();
        let peer_group = ServerGroup::join(&peer, &add.welcome).unwrap();
        let target = if art {
            StudioTarget::Flipnote {
                channel: [3; 16],
                object: [7; 16],
            }
        } else {
            StudioTarget::Index { channel: [3; 16] }
        };
        let mut source = StudioEpoch::new(&group, target, owner.device_id()).unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(854);
        let logical = target.document(&group.group_id()).unwrap();
        let body = if art {
            FlipnoteOp::SetHeader(FlipnoteHeader::Title("seed title".into()))
                .encode()
                .unwrap()
        } else {
            IndexOp::PutObject {
                object: [7; 16],
                kind: StudioKind::Flipnote,
                title: "seed title".into(),
                created_by: owner.device_id(),
                ts: 1,
                expiry: StudioExpiry::Never,
            }
            .encode()
            .unwrap()
        };
        source
            .edit_or_reseal(
                &owner,
                &group,
                &mut rng,
                &DomainOp {
                    doc_type: logical.doc_type,
                    logical_key: logical.logical_key.clone(),
                    body,
                    nonce: [0; 16],
                },
                1,
            )
            .unwrap();
        let seed = source.projection().unwrap().checkpoint([7; 32]).unwrap();
        let opening = Receipt::sign(
            logical.clone(),
            0,
            [7; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            &owner,
        )
        .unwrap();
        source = StudioEpoch::from_checkpoint(
            &group,
            target,
            owner.device_id(),
            opening,
            0,
            seed.bytes(),
        )
        .unwrap();
        // The candidate carries the same bytes under a receipt signed by an outsider. Only the
        // fixture's separate source opening was verified; none of that authority enters parsing.
        let receipt = Receipt::sign(
            logical,
            0,
            [7; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            &outsider,
        )
        .unwrap();
        assert!(receipt.verify_current_owner(&group, 0).is_err());
        Self {
            owner,
            peer,
            outsider,
            group,
            peer_group,
            target,
            source,
            receipt,
            seed,
            rng,
        }
    }
    fn candidate(&self) -> UnconfirmedStudioSeed {
        UnconfirmedStudioSeed::parse(self.target, &self.receipt, self.seed.bytes()).unwrap()
    }
    fn edit(&mut self, n: u8) -> SealedOp {
        let logical = self.target.document(&self.group.group_id()).unwrap();
        let body = match self.target {
            StudioTarget::Flipnote { .. } => {
                FlipnoteOp::SetHeader(FlipnoteHeader::Title(format!("tail {n}")))
                    .encode()
                    .unwrap()
            }
            _ => IndexOp::SetTitle {
                object: [7; 16],
                title: format!("tail {n}"),
            }
            .encode()
            .unwrap(),
        };
        self.source
            .edit_or_reseal(
                &self.owner,
                &self.group,
                &mut self.rng,
                &DomainOp {
                    doc_type: logical.doc_type,
                    logical_key: logical.logical_key,
                    body,
                    nonce: [n; 16],
                },
                1,
            )
            .unwrap()
    }
    fn open(&self, op: &SealedOp) -> SignedOp {
        let key = self
            .group
            .channel_secret(&self.owner, op.doc_type, op.doc_id)
            .unwrap();
        op.open(&key).unwrap()
    }
    fn seal(&mut self, op: &SignedOp) -> SealedOp {
        SealedOp::seal(op, &self.group, &self.owner, &mut self.rng).unwrap()
    }
}

#[test]
fn provisional_tail_replays_real_index_and_flipnote_changes_and_deduplicates() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let mut preview = f.candidate();
        let first = f.edit(1);
        preview = preview
            .prepare_tail(vec![first.clone(), first], &f.group, &f.owner)
            .unwrap()
            .prepare()
            .unwrap();
        assert_eq!(preview.applied.len(), 1);
        assert_eq!(preview.projection(), &f.source.projection().unwrap());
        for n in 2..5 {
            let op = f.edit(n);
            preview = preview
                .prepare_tail(vec![op], &f.group, &f.owner)
                .unwrap()
                .prepare()
                .unwrap();
        }
        assert_eq!(preview.projection(), &f.source.projection().unwrap());
        assert_eq!(preview.doc_id(), f.source.doc_id());
        assert_eq!(preview.applied.len(), 4);
    }
}

#[test]
fn provisional_tail_rejects_authenticated_relay_forgery_scope_and_typed_semantics() {
    for bad in [
        "signature",
        "outsider",
        "actor",
        "outer",
        "inner",
        "epoch",
        "logical",
        "body",
        "marker",
    ] {
        let mut f = Fixture::new(true);
        let sealed = f.edit(1);
        let mut op = f.open(&sealed);
        let mut domain = op.parsed_domain_op().unwrap().unwrap();
        match bad {
            "signature" => op.signature[0] ^= 1,
            "outsider" => {
                op = SignedOp::sign_domain(
                    &f.outsider,
                    op.doc_type,
                    op.doc_id,
                    op.delta.clone(),
                    &domain,
                )
                .unwrap()
            }
            "actor" => {
                op = SignedOp::sign_domain(
                    &f.peer,
                    op.doc_type,
                    op.doc_id,
                    op.delta.clone(),
                    &domain,
                )
                .unwrap()
            }
            "inner" => {
                op = SignedOp::sign_domain(
                    &f.owner,
                    op.doc_type,
                    op.doc_id + 1,
                    op.delta.clone(),
                    &domain,
                )
                .unwrap()
            }
            "logical" | "body" | "marker" => {
                if bad == "logical" {
                    domain.logical_key = vec![6; 16];
                }
                if bad == "body" {
                    domain.body = FlipnoteOp::SetHeader(FlipnoteHeader::Title(
                        "substituted semantics".into(),
                    ))
                    .encode()
                    .unwrap();
                }
                if bad == "marker" {
                    domain.nonce = [8; 16];
                }
                op = SignedOp::sign_domain(
                    &f.owner,
                    op.doc_type,
                    op.doc_id,
                    op.delta.clone(),
                    &domain,
                )
                .unwrap();
            }
            _ => {}
        }
        let mut sealed = f.seal(&op);
        if bad == "outer" {
            sealed.doc_id += 1;
        }
        if bad == "inner" {
            sealed.doc_id = f.source.doc_id();
        }
        if bad == "epoch" {
            sealed.epoch -= 1;
        }
        let result = f
            .candidate()
            .prepare_tail(vec![sealed], &f.group, &f.owner)
            .and_then(|p| p.prepare());
        assert!(result.is_err(), "accepted {bad}");
    }
}

#[test]
fn provisional_tail_requires_seed_ancestry_and_discards_a_partly_valid_page() {
    let mut f = Fixture::new(true);
    let first = f.edit(1);
    let second = f.edit(2);
    assert!(matches!(
        f.candidate()
            .prepare_tail(vec![second.clone()], &f.group, &f.owner)
            .unwrap()
            .prepare(),
        Err(ReplError::EpochScope)
    ));
    let mut forged = f.open(&second);
    let domain = forged.parsed_domain_op().unwrap().unwrap();
    let mut orphan = AutoCommit::new().with_actor(automerge::ActorId::from(
        f.owner.device_id().as_bytes().as_slice(),
    ));
    orphan.put(ROOT, "orphan", "root").unwrap();
    orphan.commit();
    forged = SignedOp::sign_domain(
        &f.owner,
        forged.doc_type,
        forged.doc_id,
        orphan.get_last_local_change().unwrap().raw_bytes().to_vec(),
        &domain,
    )
    .unwrap();
    let orphan = f.seal(&forged);
    assert!(matches!(
        f.candidate()
            .prepare_tail(vec![first, orphan], &f.group, &f.owner)
            .unwrap()
            .prepare(),
        Err(ReplError::EpochScope)
    ));
}

#[test]
fn provisional_tail_rejects_removed_authors_and_bounds_retained_work() {
    let mut f = Fixture::new(true);
    let op = f.edit(1);
    assert!(matches!(
        f.candidate()
            .prepare_tail(vec![op.clone(); 33], &f.group, &f.owner),
        Err(ReplError::EpochBound)
    ));
    let mut candidate = f.candidate();
    candidate.encoded_bytes = crate::epoch::MAX_EPOCH_BYTES;
    assert!(matches!(
        candidate
            .prepare_tail(vec![op.clone()], &f.group, &f.owner)
            .unwrap()
            .prepare(),
        Err(ReplError::EpochBound)
    ));
    let mut branch = StudioEpoch::restore(
        &f.source.snapshot().unwrap(),
        &f.peer_group,
        f.target,
        f.peer.device_id(),
    )
    .unwrap();
    let logical = f.target.document(&f.group.group_id()).unwrap();
    let authored = branch
        .edit_or_reseal(
            &f.peer,
            &f.peer_group,
            &mut f.rng,
            &DomainOp {
                doc_type: logical.doc_type,
                logical_key: logical.logical_key,
                body: FlipnoteOp::SetHeader(FlipnoteHeader::Title("removed author".into()))
                    .encode()
                    .unwrap(),
                nonce: [9; 16],
            },
            1,
        )
        .unwrap();
    let removed = f.open(&authored);
    // First prove this exact operation passes full parsing while its author is still admitted.
    f.candidate()
        .prepare_tail(vec![op.clone(), authored], &f.group, &f.owner)
        .unwrap()
        .prepare()
        .unwrap();
    f.group
        .remove_member(&f.owner, &f.peer.device_id())
        .unwrap();
    let resealed = f.seal(&removed);
    assert!(matches!(
        f.candidate()
            .prepare_tail(vec![resealed], &f.group, &f.owner),
        Err(ReplError::EpochAuthority)
    ));
}
