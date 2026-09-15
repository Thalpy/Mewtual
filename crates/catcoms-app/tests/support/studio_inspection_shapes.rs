//! Codec fixture, not signed admission. Assemble the baseline payload, then require the public
//! typed reader and canonical checkpoint round trip to validate its complete shape. This avoids
//! thousands of unrelated Automerge setup writes in a maximum-output test.
use super::app::DeviceId;
use automerge::{transaction::Transactable, AutoCommit, ROOT};
use catcoms_replication::{studio::*, DomainOp, LogicalDocument};
fn bytes(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value);
}
fn count(out: &mut Vec<u8>, value: usize) {
    out.extend_from_slice(&(value as u32).to_be_bytes());
}
fn number(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}
type Source = ([u8; 32], DeviceId, [u8; 16]);
fn sources(logical: &LogicalDocument, nonce: u128, size: usize) -> Vec<Source> {
    let mut values: Vec<_> = (1..=size)
        .map(|a| {
            let author = DeviceId::from_bytes([a as u8; 32]);
            let op = DomainOp {
                doc_type: logical.doc_type,
                logical_key: logical.logical_key.clone(),
                body: vec![],
                nonce: nonce.to_be_bytes(),
            };
            (op.id(&author), author, op.nonce)
        })
        .collect();
    values.sort_by_key(|v| v.0);
    values
}
fn source(out: &mut Vec<u8>, value: &Source, art: bool) {
    bytes(out, value.1.as_bytes());
    bytes(out, &value.2);
    if art {
        number(out, 1234);
    }
}
fn optional(out: &mut Vec<u8>, value: Option<&[u8]>) {
    out.push(u8::from(value.is_some()));
    if let Some(value) = value {
        bytes(out, value);
    }
}
fn blob(out: &mut Vec<u8>, value: &Source) {
    bytes(out, value.1.as_bytes());
    number(out, 100);
}
fn element(n: u128) -> [u8; 16] {
    if n == 1 {
        return [1; 16];
    }
    let mut id = [2; 16];
    id[14..].copy_from_slice(&(n as u16).to_be_bytes());
    id
}
pub(super) fn maximal(group: &[u8], target: StudioTarget) -> StudioProjection {
    let logical = target.document(group).unwrap();
    let art = matches!(target, StudioTarget::Flipnote { .. });
    let mut payload = vec![1];
    bytes(&mut payload, group);
    number(&mut payload, logical.doc_type.tag().into());
    bytes(&mut payload, &logical.logical_key);
    bytes(&mut payload, &target.channel());
    number(&mut payload, 1);
    if art {
        payload.extend_from_slice(&[0, 0]); // absent title/fps registers
        count(&mut payload, 999);
        let mut previous: Option<[u8; 16]> = None;
        let mut anchor: Option<[u8; 32]> = None;
        for n in 1..=999 {
            let id = element(n);
            bytes(&mut payload, &id);
            // First 512 frames consume both insertion and pixel fields: exactly 1024.
            let size = if n <= 512 { 4 } else { 1 };
            let insertions = sources(&logical, n, size);
            count(&mut payload, size);
            for value in &insertions {
                source(&mut payload, value, true);
                payload.push(1);
                optional(&mut payload, previous.as_ref().map(|v| v.as_slice()));
                optional(&mut payload, anchor.as_ref().map(|v| v.as_slice()));
                optional(&mut payload, None);
                blob(&mut payload, value);
            }
            let pixels = sources(&logical, n + 2000, size);
            source(&mut payload, &pixels[0], true);
            blob(&mut payload, &pixels[0]);
            count(&mut payload, size - 1);
            for value in &pixels[1..] {
                source(&mut payload, value, true);
                blob(&mut payload, value);
            }
            previous = Some(id);
            anchor = Some(insertions[0].0);
        }
        count(&mut payload, 0);
    } else {
        count(&mut payload, 64);
        let title = "雪\n\"".repeat(32);
        for n in 1..=64 {
            bytes(&mut payload, &element(n));
            count(&mut payload, 4);
            for value in sources(&logical, n, 4) {
                source(&mut payload, &value, false);
                payload.push(0);
                bytes(&mut payload, title.as_bytes());
                bytes(&mut payload, value.1.as_bytes());
                number(&mut payload, 1234);
                payload.push(1);
            }
            for field in [2000, 4000] {
                let values = sources(&logical, n + field, 4);
                for (i, value) in values.iter().enumerate() {
                    source(&mut payload, value, false);
                    if field == 2000 {
                        bytes(&mut payload, title.as_bytes());
                    } else {
                        payload.push(2);
                        number(&mut payload, 9_007_199_254_740_991);
                    }
                    if i == 0 {
                        count(&mut payload, 3);
                    }
                }
            }
        }
        for _ in 0..3 {
            count(&mut payload, 0);
        } // overflow, deleted objects, tombstones
    }
    let mut doc = AutoCommit::new();
    doc.put(ROOT, "v", 1u64).unwrap();
    doc.put(ROOT, "kind", if art { "flipnote" } else { "index" })
        .unwrap();
    doc.put(ROOT, "channel", hex::encode(target.channel()))
        .unwrap();
    doc.put(ROOT, "epoch", 1u64).unwrap();
    if art {
        doc.put(ROOT, "id", hex::encode(&logical.logical_key))
            .unwrap();
        doc.put(ROOT, "w", 192u64).unwrap();
        doc.put(ROOT, "h", 144u64).unwrap();
    }
    doc.put(ROOT, "_studio/seed", payload).unwrap();
    doc.commit();
    let mut projection = if art {
        StudioProjection::Flipnote(Box::new(
            FlipnoteFrameProjection::read(&logical, target.channel(), 1, &doc).unwrap(),
        ))
    } else {
        StudioProjection::Index(StudioIndexProjection::read(&logical, 1, &doc).unwrap())
    };
    match &mut projection {
        StudioProjection::Index(p) => p.epoch = 0,
        StudioProjection::Flipnote(p) => p.epoch = 0,
    }
    let seed = projection.checkpoint([7; 32]).unwrap();
    eprintln!(
        "INSPECTION_SHAPE art={art} canonical_seed_bytes={}",
        seed.bytes().len()
    );
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
        assert!(p
            .frames
            .values()
            .all(|f| f.insertions.len() <= 4 && f.pixels.conflicts.len() <= 3));
        StudioProjection::Flipnote(Box::new(p))
    } else {
        let p = StudioIndexProjection::read(&logical, 1, &canonical).unwrap();
        assert_eq!(p.objects.len(), 64);
        assert!(p.overflow.is_empty());
        assert!(p.objects.contains_key(&[1; 16]));
        assert!(p.objects.values().all(|o| o.creations.len() == 4
            && o.title.conflicts.len() == 3
            && o.expiry.conflicts.len() == 3));
        StudioProjection::Index(p)
    }
}
