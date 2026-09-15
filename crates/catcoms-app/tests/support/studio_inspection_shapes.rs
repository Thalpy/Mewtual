//! Codec shape fixture, not signed admission: maximum playable entries and retained conflicts.
//! The public typed reader and canonical checkpoint codec validate all constructed records.
use automerge::{transaction::Transactable, ActorId, AutoCommit, ROOT};
use catcoms_replication::{studio::*, DomainOp};

pub(super) fn maximal(group: &[u8], target: StudioTarget) -> StudioProjection {
    let logical = target.document(group).unwrap();
    let art = matches!(target, StudioTarget::Flipnote { .. });
    let mut base = AutoCommit::new();
    base.put(ROOT, "v", 1u64).unwrap();
    base.put(ROOT, "kind", if art { "flipnote" } else { "index" })
        .unwrap();
    base.put(ROOT, "channel", hex::encode(target.channel()))
        .unwrap();
    base.put(ROOT, "epoch", 0u64).unwrap();
    if art {
        base.put(ROOT, "id", hex::encode(&logical.logical_key))
            .unwrap();
        base.put(ROOT, "w", 192u64).unwrap();
        base.put(ROOT, "h", 144u64).unwrap();
    }
    base.commit();
    let mut merged = base.clone();
    for a in 1..=6 {
        let author = super::app::DeviceId::from_bytes([a; 32]);
        let mut branch = base.fork();
        branch.set_actor(ActorId::from(author.as_bytes().to_vec()));
        for n in 1u128..=if art { 1000 } else { 65 } {
            let element = if n == 1 { [1; 16] } else { n.to_be_bytes() };
            let insert = if art {
                FlipnoteOp::InsertFrame {
                    frame: element,
                    after: None,
                    cid: [a; 32],
                    bytes: 100,
                }
                .encode()
                .unwrap()
            } else {
                IndexOp::PutObject {
                    object: element,
                    kind: StudioKind::Flipnote,
                    title: "雪\n\"".repeat(32),
                    created_by: author,
                    ts: 1234,
                    expiry: StudioExpiry::Never,
                }
                .encode()
                .unwrap()
            };
            let edits = if art {
                vec![
                    (insert, "i", n),
                    (
                        FlipnoteOp::ReplaceFrame {
                            frame: element,
                            cid: [a; 32],
                            bytes: 100,
                        }
                        .encode()
                        .unwrap(),
                        "r",
                        n + 2000,
                    ),
                ]
            } else {
                vec![
                    (insert, "i", n),
                    (
                        IndexOp::SetTitle {
                            object: element,
                            title: "雪\n\"".repeat(32),
                        }
                        .encode()
                        .unwrap(),
                        "t",
                        n + 2000,
                    ),
                    (
                        IndexOp::SetExpiry {
                            object: element,
                            expiry: StudioExpiry::At(9_007_199_254_740_991),
                        }
                        .encode()
                        .unwrap(),
                        "e",
                        n + 4000,
                    ),
                ]
            };
            for (body, tag, nonce) in edits {
                let domain = DomainOp {
                    doc_type: logical.doc_type,
                    logical_key: logical.logical_key.clone(),
                    body,
                    nonce: nonce.to_be_bytes(),
                };
                let id = hex::encode(domain.id(&author));
                let key = if tag == "i" {
                    format!("i/{}/{id}", hex::encode(element))
                } else {
                    format!("{tag}/{}", hex::encode(element))
                };
                let mut record = vec![1];
                record.extend_from_slice(author.as_bytes());
                if art {
                    record.extend_from_slice(&1234u64.to_be_bytes());
                    record.extend_from_slice(&[0, 0]);
                }
                record.extend_from_slice(&domain.encode().unwrap());
                branch.put(ROOT, key, record).unwrap();
                branch.put(ROOT, format!("_p1/op/{id}"), 1u64).unwrap();
            }
        }
        branch.commit();
        merged.merge(&mut branch).unwrap();
    }
    let original = if art {
        StudioProjection::Flipnote(Box::new(
            FlipnoteFrameProjection::read(&logical, target.channel(), 0, &merged).unwrap(),
        ))
    } else {
        StudioProjection::Index(StudioIndexProjection::read(&logical, 0, &merged).unwrap())
    };
    let seed = original.checkpoint([7; 32]).unwrap();
    assert!(seed.bytes().len() <= catcoms_replication::MAX_CHECKPOINT_BYTES);
    let mut canonical = AutoCommit::new();
    canonical
        .apply_changes([automerge::Change::from_bytes(seed.bytes().to_vec()).unwrap()])
        .unwrap();
    if art {
        let p = FlipnoteFrameProjection::read(&logical, target.channel(), 1, &canonical).unwrap();
        assert_eq!(p.frames.len(), 999);
        assert!(p.over_cap.is_empty());
        assert_eq!(
            p.frames
                .values()
                .map(|f| usize::from(f.insertions.len() > 1)
                    + usize::from(!f.pixels.conflicts.is_empty()))
                .sum::<usize>(),
            1024
        );
        StudioProjection::Flipnote(Box::new(p))
    } else {
        let p = StudioIndexProjection::read(&logical, 1, &canonical).unwrap();
        assert_eq!(p.objects.len(), 64);
        assert!(p.overflow.is_empty());
        assert!(p.objects.values().all(|o| o.creations.len() == 4
            && o.title.conflicts.len() == 3
            && o.expiry.conflicts.len() == 3));
        StudioProjection::Index(p)
    }
}
