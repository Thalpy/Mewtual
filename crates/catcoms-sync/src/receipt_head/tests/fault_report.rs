//! W-1: the scoped head query's v2 report section. v1 bytes are unchanged; a report is always a
//! complete pair, decoded only at admission, and nothing about it is authority.
use super::*;
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_replication::studio::StudioTarget;

fn pair(document: &LogicalDocument, owner: &MlsDevice) -> [Receipt; 2] {
    [7u8, 8].map(|close| {
        Receipt::sign(
            document.clone(),
            0,
            [close; 32],
            [close; 32],
            0,
            InheritedCheckpoint::EpochZero,
            owner,
        )
        .unwrap()
    })
}

#[test]
fn a_report_is_v2_complete_bounded_and_scoped_while_v1_bytes_are_unchanged() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let gid = group.group_id();
    for target in [
        CheckpointTarget::Studio(StudioTarget::Index { channel: [3; 16] }),
        CheckpointTarget::Studio(StudioTarget::Flipnote {
            channel: [3; 16],
            object: [7; 16],
        }),
        CheckpointTarget::Registry(5),
    ] {
        let kind = target.head_kind();
        let document = target.document(&gid).unwrap();
        let report = pair(&document, &owner);
        let v1 = encode_scoped_query(target, &gid, [9; 16], None).unwrap();
        assert_eq!(v1[0], 1, "no report means exactly the v1 bytes");
        assert_eq!(
            decode_scoped_query(kind, &v1, &gid).unwrap(),
            (target, [9; 16], None)
        );
        let v2 = encode_scoped_query(target, &gid, [9; 16], Some(&report)).unwrap();
        assert_eq!(v2[0], 2);
        assert!(v2.len() <= MAX_QUERY_V2);
        let (decoded, nonce, bytes) = decode_scoped_query(kind, &v2, &gid).unwrap();
        assert_eq!((decoded, nonce), (target, [9; 16]));
        let bytes = bytes.expect("v2 carries a report section");
        assert_eq!(
            decode_fault_report(&bytes, &document).unwrap(),
            Some(report.clone())
        );
        // One receipt is never a report: the pair is always complete.
        let mut one = bytes.clone();
        one[0] = 1;
        assert!(decode_fault_report(&one, &document).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_fault_report(&trailing, &document).is_err());
        // A report for another document is refused at admission decode.
        let other = CheckpointTarget::Registry(6).document(&gid).unwrap();
        assert!(decode_fault_report(&bytes, &other).is_err());
        assert_eq!(decode_fault_report(&[0], &document).unwrap(), None);
        // The header must still be canonical v1 fields, and the section must exist and fit.
        let mut header_only = v2.clone();
        header_only.truncate(v2.len() - bytes.len());
        assert!(decode_scoped_query(kind, &header_only, &gid).is_err());
        let mut oversized = v2.clone();
        oversized.resize(MAX_QUERY_V2 + 1, 0);
        assert!(decode_scoped_query(kind, &oversized, &gid).is_err());
    }
    // A report for a different target than the query names cannot be encoded at all.
    let target = CheckpointTarget::Registry(5);
    let wrong = pair(
        &CheckpointTarget::Registry(6).document(&gid).unwrap(),
        &owner,
    );
    assert!(encode_scoped_query(target, &gid, [9; 16], Some(&wrong)).is_err());
}
