use super::*;
use catcoms_replication::InheritedCheckpoint;

#[test]
fn owner_decision_extension_is_exact_bounded_and_preserves_legacy_encoding() {
    let owner = catcoms_mls::MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let doc = LogicalDocument::new(
        group.group_id(),
        catcoms_wire::DocType::StudioObject,
        b"private-cat".to_vec(),
    )
    .unwrap();
    let close = CloseRecord::sign(&doc, 4, 0, vec![[1; 32], [2; 32]], &owner).unwrap();
    let receipt = Receipt::sign(
        doc.clone(),
        0,
        close.hash(),
        [3; 32],
        0,
        InheritedCheckpoint::EpochZero,
        &owner,
    )
    .unwrap();
    let mut state = EpochOwnerReceiptState::default();
    state.journal.prepare(receipt.clone(), &group, 0).unwrap();
    let scope = scope_bytes(7, &doc).unwrap();
    let legacy = state.encode(&scope, &doc).unwrap();
    let restored = EpochOwnerReceiptState::decode(&legacy, &scope, &doc).unwrap();
    assert!(restored.close_for(&receipt).is_none());
    assert_eq!(*restored.encode(&scope, &doc).unwrap(), *legacy);
    state.decision_close = Some((receipt.hash(), close.clone()));
    let extended = state.encode(&scope, &doc).unwrap();
    assert_eq!(&extended[..legacy.len()], legacy.as_slice());
    assert_eq!(extended[legacy.len()], 2);
    let restored = EpochOwnerReceiptState::decode(&extended, &scope, &doc).unwrap();
    assert_eq!(
        restored.close_for(&receipt).unwrap().encode(),
        close.encode()
    );
    // A complete legacy prefix is intentionally valid. Every partial extension must reject.
    for end in legacy.len() + 1..extended.len() {
        assert!(EpochOwnerReceiptState::decode(&extended[..end], &scope, &doc).is_err());
    }
    for offset in [legacy.len(), legacy.len() + 5, extended.len() - 65] {
        let mut bad = extended.to_vec();
        bad[offset] ^= 1;
        assert!(EpochOwnerReceiptState::decode(&bad, &scope, &doc).is_err());
    }
    let mut trailing = extended.to_vec();
    trailing.push(0);
    assert!(EpochOwnerReceiptState::decode(&trailing, &scope, &doc).is_err());
    assert!(EpochOwnerReceiptState::decode(&vec![0; MAX_RECORD_BYTES + 1], &scope, &doc).is_err());
    state.journal.mark_published(receipt.hash()).unwrap();
    let saved = state.encode(&scope, &doc).unwrap();
    assert_eq!(
        EpochOwnerReceiptState::decode(&saved, &scope, &doc)
            .unwrap()
            .close_for(&receipt)
            .unwrap()
            .encode(),
        close.encode()
    );
}
