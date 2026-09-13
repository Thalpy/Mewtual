use super::*;
use crate::{CheckpointSeed, InheritedCheckpoint};
use automerge::transaction::Transactable;
use automerge::{AutoCommit, Change, ReadDoc, Value, ROOT};
use catcoms_mls::MlsDevice;
mod tail;

#[test]
fn provisional_seed_rejects_noncanonical_order_after_raw_and_typed_checks_pass() {
    let signer = MlsDevice::generate().unwrap();
    for target in [
        StudioTarget::Index { channel: [3; 16] },
        StudioTarget::Flipnote {
            channel: [3; 16],
            object: [7; 16],
        },
    ] {
        let logical = target.document(b"provisional-encoding-test").unwrap();
        let projection = target.read(&logical, 0, &AutoCommit::new()).unwrap();
        let canonical = projection.checkpoint([7; 32]).unwrap();
        let mut source = AutoCommit::new();
        source
            .apply_changes([Change::from_bytes(canonical.bytes().to_vec()).unwrap()])
            .unwrap();
        let keys: Vec<_> = source.keys(ROOT).collect();
        assert!(keys.len() > 1, "fixture must change operation ordering");
        let reordered = CheckpointSeed::build(&logical, 1, [7; 32], |doc| {
            for key in keys.iter().rev() {
                let Some((Value::Scalar(value), _)) = source.get(ROOT, key).unwrap() else {
                    panic!("canonical Studio root must be scalar");
                };
                doc.put(ROOT, key, value.into_owned()).unwrap();
            }
            Ok(())
        })
        .unwrap();
        assert_ne!(reordered.bytes(), canonical.bytes());
        let receipt = Receipt::sign(
            logical.clone(),
            0,
            [7; 32],
            reordered.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            &signer,
        )
        .unwrap();
        // These are the actual earlier production gates. The fixture must reach the final
        // compact re-encoding comparison, rather than fail at signature, actor or root schema.
        let raw_valid =
            crate::checkpoint::inspect_unconfirmed_seed(&receipt, reordered.bytes()).unwrap();
        assert_eq!(
            target.read(&logical, 1, &raw_valid).unwrap(),
            target.read(&logical, 1, &source).unwrap()
        );
        assert!(UnconfirmedStudioSeed::parse(target, &receipt, reordered.bytes()).is_err());
    }
}
