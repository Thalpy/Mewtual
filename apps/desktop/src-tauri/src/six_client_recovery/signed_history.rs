//! Test-only inspection of the existing sealed-snapshot payload. Only the native fixture's
//! requested channel log escapes this parser; MLS/routing/signer secrets are borrowed and skipped.
//! Snapshot bytes stay inside their Zeroizing owner and never enter traces or panic messages.
use super::*;
use catcoms_replication::SignedOp;
use catcoms_wire::{Decoder, DocType};

pub(super) fn channel_operations(snapshot: &[u8], channel: u128) -> BTreeMap<[u8; 32], SignedOp> {
    let mut root = Decoder::new(snapshot);
    for _ in 0..3 {
        root.get_bytes()
            .expect("snapshot MLS/routing/invite prefix");
    }
    let count = root.get_u32().expect("snapshot document count");
    assert!(count <= 256, "small fixture document bound exceeded");
    let mut result = None;
    for _ in 0..count {
        let mut doc = Decoder::new(root.get_bytes().expect("snapshot document"));
        let tag = doc.get_u16().expect("document type");
        let id = doc.get_u128().expect("document identity");
        if tag != DocType::Channel.tag() || id != channel {
            continue;
        }
        assert!(result.is_none(), "one exact channel snapshot");
        doc.get_bytes().expect("materialized document");
        let operations = doc.get_u32().expect("original signed log count");
        assert!(operations <= 1024, "small fixture operation bound exceeded");
        let mut log = BTreeMap::new();
        for _ in 0..operations {
            let bytes = doc.get_bytes().expect("original signed operation");
            let op = SignedOp::decode(bytes).expect("canonical original operation");
            assert!(op.verify(), "original full device/signature verification");
            assert_eq!(op.doc_type, DocType::Channel);
            assert_eq!(op.doc_id, channel);
            assert_eq!(op.encode(), bytes, "exact canonical original bytes");
            assert!(
                log.insert(op.hash(), op).is_none(),
                "duplicate original operation"
            );
        }
        doc.finish()
            .expect("legacy chat snapshot has no P1 extension");
        result = Some(log);
    }
    result.expect("actor opened the exact channel")
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_wire::Encoder;

    fn operation(device: &MlsDevice, channel: u128) -> SignedOp {
        SignedOp::sign(
            device,
            DocType::Channel,
            channel,
            b"fixed fixture change".to_vec(),
        )
        .unwrap()
    }
    // Exercise only the snapshot-log observation contract. No fake document or signed operation
    // created here is ever supplied to a running recovery actor.
    fn snapshot(operations: &[SignedOp]) -> Vec<u8> {
        let mut doc = Encoder::new();
        doc.put_u16(DocType::Channel.tag());
        doc.put_u128(channel_id("general"));
        doc.put_bytes(&[]).unwrap();
        doc.put_u32(operations.len() as u32);
        for op in operations {
            doc.put_bytes(&op.encode()).unwrap();
        }
        let mut root = Encoder::new();
        for _ in 0..3 {
            root.put_bytes(&[]).unwrap();
        }
        root.put_u32(1);
        root.put_bytes(&doc.finish()).unwrap();
        root.finish()
    }
    #[test]
    fn observation_rejects_corrupt_signature_wrong_document_and_duplicate_operations() {
        let device = MlsDevice::generate().unwrap();
        let channel = channel_id("general");
        let valid = operation(&device, channel);
        let parsed = channel_operations(&snapshot(std::slice::from_ref(&valid)), channel);
        assert_eq!(parsed.get(&valid.hash()), Some(&valid));
        let mut corrupt = valid.clone();
        corrupt.signature[0] ^= 1;
        let wrong_scope = operation(&device, channel + 1);
        for invalid in [vec![corrupt], vec![wrong_scope], vec![valid.clone(), valid]] {
            assert!(
                std::panic::catch_unwind(|| channel_operations(&snapshot(&invalid), channel))
                    .is_err()
            );
        }
    }
    #[test]
    fn identical_rendered_messages_cannot_hide_missing_or_reauthored_operations() {
        let device = MlsDevice::generate().unwrap();
        let original = operation(&device, channel_id("general"));
        let messages = BTreeMap::from([(
            "fixture message".into(),
            ("display author".into(), "same text".into()),
        )]);
        let expected = History {
            messages: messages.clone(),
            operations: BTreeMap::from([(original.hash(), original)]),
        };
        let missing = History {
            messages: messages.clone(),
            operations: BTreeMap::new(),
        };
        assert_ne!(expected, missing);
        let replacement = operation(&MlsDevice::generate().unwrap(), channel_id("general"));
        let reauthored = History {
            messages,
            operations: BTreeMap::from([(replacement.hash(), replacement)]),
        };
        assert_ne!(expected, reauthored);
    }
}
